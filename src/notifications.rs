use anyhow::Context as _;
use chrono::{Local, Utc};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 通知服务抽象：便于测试注入 Mock 捕获调用。
pub trait NotificationService: Send + Sync {
    fn notify(&self, title: &str, body: &str) -> anyhow::Result<()>;
}

/// 桌面通知实现（notify-rust）。
pub struct DesktopNotificationService;

/// macOS：首次通知前完成 UNUserNotificationCenter 授权请求。
/// 旧 NSUserNotification（legacy）路径在 macOS 26 上被 usernoted 以
/// "Legacy client … Denying message 3" 拒绝，通知永远无法投递（且首次
/// 发送前未设置应用身份会弹 "Where is use_default?" 选择框）。
/// `preview-macos-un` 后端改走 UNUserNotificationCenter：
/// - 需要 .app bundle（Info.plist 提供 bundle id，.app 分发天然满足）；
/// - 需要一次授权请求；授权被拒或未打包运行时静默降级，不再弹窗、不阻塞。
#[cfg(target_os = "macos")]
fn claim_notification_identity() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // request_auth_blocking 会等待用户响应系统授权弹窗（可能很久），
        // 必须在后台线程执行，避免首次 tick 阻塞主线程；授权完成前的
        // 首条通知可能被拒（UI 显示错误 Toast），后续通知正常投递。
        std::thread::spawn(|| {
            let _ = notify_rust::request_auth_blocking();
        });
    });
}

/// 通知图标：将内嵌 PNG 惰性写入配置目录，Linux 通知守护进程经
/// `app_icon` 绝对路径显示应用图标（主题名在未安装图标的开发环境不可用）。
/// macOS 图标来自 .app bundle（`icon()` 无效但无害），Windows 由系统通知实现决定。
fn notification_icon_path() -> Option<std::path::PathBuf> {
    static ICON: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    ICON.get_or_init(|| {
        let dir = crate::persistence::config_path().parent()?.to_path_buf();
        let path = dir.join("notification-icon.png");
        if !path.exists() {
            std::fs::write(&path, include_bytes!("../assets/icons/quota-watch-128.png")).ok()?;
        }
        Some(path)
    })
    .clone()
}

impl NotificationService for DesktopNotificationService {
    fn notify(&self, title: &str, body: &str) -> anyhow::Result<()> {
        #[cfg(target_os = "macos")]
        claim_notification_identity();
        let mut notification = notify_rust::Notification::new();
        notification
            .summary(title)
            .body(body)
            .timeout(notify_rust::Timeout::Milliseconds(8000));
        if let Some(path) = notification_icon_path() {
            notification.icon(&path.to_string_lossy());
        }
        notification
            .show()
            .map(|_| ())
            .context("系统通知发送失败（检查通知权限 / 守护进程）")
    }
}

/// 测试用 Mock：记录调用次数与标题（按序号可断言）。
#[derive(Default)]
#[allow(dead_code)] // 仅测试使用，bin 编译时静默
pub struct MockNotificationService {
    pub calls: AtomicUsize,
    pub titles: std::sync::Mutex<Vec<String>>,
}

impl MockNotificationService {
    #[allow(dead_code)]
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
    pub fn titles_snapshot(&self) -> Vec<String> {
        self.titles.lock().unwrap().clone()
    }
}

impl NotificationService for MockNotificationService {
    fn notify(&self, title: &str, _body: &str) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.titles.lock().unwrap().push(title.to_string());
        Ok(())
    }
}

/// 按模板渲染提醒标题：替换 `{service}`、`{minutes}`、`{next_reset}`
/// （本地时区 `%Y-%m-%d %H:%M:%S`），未知占位符原样保留。
pub fn render_title(
    template: &str,
    service_name: &str,
    minutes: u64,
    next_reset: chrono::DateTime<Utc>,
) -> String {
    template
        .replace("{service}", service_name)
        .replace("{minutes}", &minutes.to_string())
        .replace(
            "{next_reset}",
            &next_reset
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        )
}

/// 通知文案：三类提醒标题分别由 `ReminderTemplate` 对应模板经
/// `render_title` 渲染（timer_engine 传入用户模板）；以下便捷函数
/// 使用默认模板，仅测试使用。
#[allow(dead_code)] // 仅测试使用；生产路径经 render_title + 用户模板
pub fn title_first_alert(
    service_name: &str,
    minutes: u32,
    next_reset: chrono::DateTime<Utc>,
) -> String {
    render_title(
        &crate::models::ReminderTemplate::default().title_first_alert,
        service_name,
        minutes as u64,
        next_reset,
    )
}

#[allow(dead_code)] // 仅测试使用；生产路径经 render_title + 用户模板
pub fn title_recurring(
    service_name: &str,
    minutes: u64,
    next_reset: chrono::DateTime<Utc>,
) -> String {
    render_title(
        &crate::models::ReminderTemplate::default().title_recurring,
        service_name,
        minutes.max(1),
        next_reset,
    )
}

#[allow(dead_code)] // 仅测试使用；生产路径经 render_title + 用户模板
pub fn title_reset(service_name: &str, next_reset: chrono::DateTime<Utc>) -> String {
    render_title(
        &crate::models::ReminderTemplate::default().title_reset,
        service_name,
        0,
        next_reset,
    )
}

pub fn body_with_next_reset(next_reset_at: chrono::DateTime<Utc>) -> String {
    format!(
        "下次重置：{}",
        next_reset_at
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn mock_records_calls() {
        let mock = MockNotificationService::default();
        assert_eq!(mock.call_count(), 0);
        mock.notify("t", "b").unwrap();
        mock.notify("t", "b").unwrap();
        assert_eq!(mock.call_count(), 2);
    }

    #[test]
    fn notification_icon_file_written() {
        // 惰性写出内嵌 PNG 到配置目录；路径存在且为非空 PNG（魔数校验）
        let path = notification_icon_path().expect("图标路径应可解析");
        let bytes = std::fs::read(&path).expect("图标文件应已写出");
        assert!(bytes.len() > 8);
        assert_eq!(&bytes[..4], b"\x89PNG");
    }

    #[test]
    fn message_texts() {
        let next = Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        assert_eq!(
            title_first_alert("openai", 30, next),
            "[openai] 将在 30 分钟后重置"
        );
        assert_eq!(
            title_recurring("openai", 2, next),
            "[openai] 即将重置！还剩 2 分钟"
        );
        assert_eq!(title_reset("gpt", next), "[gpt] 配额已重置");
    }

    #[test]
    fn render_title_replaces_placeholders() {
        let next = Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        assert_eq!(
            render_title(
                "【{service}】只剩 {minutes} 分钟，{next_reset} 重置",
                "openai",
                27,
                next
            ),
            format!(
                "【openai】只剩 27 分钟，{} 重置",
                next.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S")
            )
        );
    }

    #[test]
    fn render_title_without_placeholders_keeps_text() {
        let next = Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        assert_eq!(render_title("快没啦", "openai", 5, next), "快没啦");
    }

    #[test]
    fn render_title_unknown_placeholder_kept_verbatim() {
        let next = Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        assert_eq!(
            render_title("{Service} {foo}", "openai", 5, next),
            "{Service} {foo}"
        );
    }

    #[test]
    fn body_local_time() {
        let t = Utc.with_ymd_and_hms(2026, 10, 16, 5, 0, 0).unwrap();
        let body = body_with_next_reset(t);
        assert!(body.contains("下次重置："));
        assert!(body.contains("2026-10-16"));
    }
}
