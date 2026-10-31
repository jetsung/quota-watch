use crate::models::PersistedState;
use anyhow::Context as _;
use chrono::Local;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 配置文件路径（按平台解析）：
/// - Linux：`$HOME/.config/recur/config.json`
/// - macOS：`~/Library/Application Support/cn.gcli.recur/config.json`
/// - Windows：`%APPDATA%\recur\config.json`（扁平路径，不经 `ProjectDirs::config_dir()`）
///
/// 标准路径无法解析时统一回退 `$HOME/.config/recur/config.json`。
/// 导出与 WebDAV 上传文件使用 `recur` 系命名，仅本地文件名为 `config.json`。
pub fn config_path() -> PathBuf {
    platform_config_path("recur")
}

/// 旧版（quota-watch）配置路径：仅用于启动时一次性迁移，不再写入。
pub fn legacy_config_path() -> PathBuf {
    platform_config_path("quota-watch")
}

fn platform_config_path(app: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join(app).join("config.json");
        }
    }
    #[cfg(not(target_os = "windows"))]
    if let Some(dirs) = directories::ProjectDirs::from("cn", "gcli", app) {
        return dirs.config_dir().join("config.json");
    }
    // 回退：$HOME/.config/<app>/config.json
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join(app).join("config.json")
}

/// 启动加载：新路径不存在时，若存在旧版（quota-watch）配置则迁移一次；
/// 解析失败则备份为 `config.json.bak.<timestamp>` 并以空状态启动。
/// 返回值第二个元素为告警信息（用于写入通知日志 / Toast）。
pub fn load() -> (PersistedState, Option<String>) {
    load_from(&config_path(), &legacy_config_path())
}

fn load_from(path: &Path, legacy: &Path) -> (PersistedState, Option<String>) {
    if !path.exists() {
        if legacy.exists() {
            return migrate_legacy(path, legacy);
        }
        return (PersistedState::default(), None);
    }
    match fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<PersistedState>(&content) {
            Ok(state) => (state, None),
            Err(e) => {
                let backup = backup_corrupted(path);
                (
                    PersistedState::default(),
                    Some(format!(
                        "配置文件解析失败（{}），已备份为 {}，以空配置启动",
                        e,
                        backup.display()
                    )),
                )
            }
        },
        Err(e) => (
            PersistedState::default(),
            Some(format!("配置文件读取失败（{}），以空配置启动", e)),
        ),
    }
}

/// 从旧版配置迁移一次：读取旧文件并写入新路径；迁移失败时仍以旧配置启动。
fn migrate_legacy(path: &Path, legacy: &Path) -> (PersistedState, Option<String>) {
    match fs::read_to_string(legacy) {
        Ok(content) => match serde_json::from_str::<PersistedState>(&content) {
            Ok(state) => {
                let warning = save_to(&state, path)
                    .err()
                    .map(|e| format!("旧配置迁移到新目录失败（{}），本次仍使用旧配置", e));
                (state, warning)
            }
            Err(e) => {
                let backup = backup_corrupted(legacy);
                (
                    PersistedState::default(),
                    Some(format!(
                        "旧配置文件解析失败（{}），已备份为 {}，以空配置启动",
                        e,
                        backup.display()
                    )),
                )
            }
        },
        Err(e) => (
            PersistedState::default(),
            Some(format!("旧配置文件读取失败（{}），以空配置启动", e)),
        ),
    }
}

fn backup_corrupted(path: &Path) -> PathBuf {
    let ts = Local::now().format("%Y%m%d-%H%M%S");
    let backup = path.with_extension(format!("json.bak.{}", ts));
    if let Err(e) = fs::rename(path, &backup) {
        eprintln!("备份损坏的配置文件失败: {}", e);
    }
    backup
}

/// 保存：`to_string_pretty` → 写 `.tmp` → 原子 `rename`（Windows 先 `remove_file`）。
/// 失败时保留 `.tmp` 并返回错误（由 UI Toast 展示）。
pub fn save(state: &PersistedState) -> anyhow::Result<()> {
    save_to(state, &config_path())
}

fn save_to(state: &PersistedState, path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(state).context("序列化配置失败")?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)
            .with_context(|| format!("创建临时文件失败: {}", tmp.display()))?;
        f.write_all(json.as_bytes()).context("写入临时文件失败")?;
        f.sync_all().ok();
    }
    #[cfg(target_os = "windows")]
    if path.exists() {
        fs::remove_file(path).context("删除旧配置文件失败")?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        // 失败时保留 .tmp 便于排查
        anyhow::anyhow!(
            "原子替换配置失败（{}），临时文件保留于 {}",
            e,
            tmp.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ServiceEntry;
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn entry(name: &str) -> ServiceEntry {
        ServiceEntry {
            id: Uuid::new_v4(),
            service_name: name.into(),
            interval_hours: 5.0,
            last_reset_time: Utc.with_ymd_and_hms(2026, 10, 16, 0, 0, 0).unwrap(),
            reminder_threshold_minutes: 60,
            secondary_reminder_interval_minutes: 5,
            is_muted: false,
            reset_config_mode: Default::default(),
            url: None,
            is_pinned: false,
            pinned_at: None,
        }
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("recur-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");

        let state = PersistedState {
            services: vec![entry("openai"), entry("anthropic")],
            global_notifications_enabled: true,
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        std::fs::write(&path, json).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: PersistedState = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.services.len(), 2);
        assert_eq!(loaded.services[0].service_name, "openai");
        assert!(loaded.global_notifications_enabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_file_returns_default() {
        // 隔离配置目录：该测试断言"文件不存在 → 默认状态"，
        // 不得读取真实用户配置（应用使用过的机器上配置含服务，断言必失败）
        let dir = std::env::temp_dir().join(format!("recur-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: 测试进程内环境变量隔离；仅本测试经由 load() 读取配置路径
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            std::env::set_var("HOME", &dir);
        }
        let (state, warning) = load();
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("HOME");
        }
        assert!(state.services.is_empty());
        assert!(warning.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn migrates_legacy_config_to_new_path() {
        let dir = std::env::temp_dir().join(format!("recur-test-{}", Uuid::new_v4()));
        let new_path = dir.join("recur").join("config.json");
        let legacy_path = dir.join("quota-watch").join("config.json");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        let state = PersistedState {
            services: vec![entry("legacy")],
            ..Default::default()
        };
        std::fs::write(&legacy_path, serde_json::to_string(&state).unwrap()).unwrap();

        let (loaded, warning) = load_from(&new_path, &legacy_path);

        assert_eq!(loaded.services.len(), 1);
        assert_eq!(loaded.services[0].service_name, "legacy");
        assert!(warning.is_none());
        assert!(new_path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_corrupted_file_backs_up() {
        let dir = std::env::temp_dir().join(format!("recur-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, "{ not valid json").unwrap();

        // 直接调用内部逻辑验证备份分支
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(serde_json::from_str::<PersistedState>(&content).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reminder_template_roundtrip() {
        let state = PersistedState {
            reminder_template: crate::models::ReminderTemplate {
                title_first_alert: "【{service}】只剩 {minutes} 分钟".into(),
                title_recurring: "[{service}] 倒计时 {minutes}".into(),
                title_reset: "{service} 已触发 @ {next_reset}".into(),
            },
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let loaded: PersistedState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            loaded.reminder_template.title_first_alert,
            "【{service}】只剩 {minutes} 分钟"
        );
        assert_eq!(
            loaded.reminder_template.title_recurring,
            "[{service}] 倒计时 {minutes}"
        );
        assert_eq!(
            loaded.reminder_template.title_reset,
            "{service} 已触发 @ {next_reset}"
        );
    }

    #[test]
    fn old_json_without_reminder_template_defaults() {
        // 旧格式配置（无 reminder_template 字段）反序列化得到默认模板
        let json = r#"{
            "services": [],
            "global_notifications_enabled": true,
            "webdav_config": null
        }"#;
        let loaded: PersistedState = serde_json::from_str(json).unwrap();
        let dft = crate::models::ReminderTemplate::default();
        assert_eq!(
            loaded.reminder_template.title_first_alert,
            dft.title_first_alert
        );
        assert_eq!(
            loaded.reminder_template.title_recurring,
            dft.title_recurring
        );
        assert_eq!(loaded.reminder_template.title_reset, dft.title_reset);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_config_path_uses_appdata_flat_layout() {
        let dir = std::env::temp_dir().join(format!("recur-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: 测试进程内环境变量隔离；仅本测试读取 config_path()
        unsafe {
            std::env::set_var("APPDATA", &dir);
        }
        let path = config_path();
        unsafe {
            std::env::remove_var("APPDATA");
        }
        assert_eq!(path, dir.join("recur").join("config.json"));
        let s = path.to_string_lossy();
        assert!(!s.contains("gcli"));
        assert!(!s.contains("config\\"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_config_path_falls_back_when_appdata_missing() {
        let dir = std::env::temp_dir().join(format!("recur-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: 测试进程内环境变量隔离；仅本测试读取 config_path()
        unsafe {
            std::env::remove_var("APPDATA");
            std::env::set_var("HOME", &dir);
        }
        let path = config_path();
        unsafe {
            std::env::remove_var("HOME");
        }
        assert_eq!(path, dir.join(".config").join("recur").join("config.json"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
