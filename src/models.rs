use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// 重置时间配置模式：仅影响表单输入语义，不影响 `next_reset_at` 计算。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResetConfigMode {
    /// 上次重置时间 + 间隔（现有模式）。
    #[default]
    LastReset,
    /// 直接输入下次重置的绝对时间点（按全局时区解释）。
    NextReset,
    /// 输入距下次重置的剩余时长（以 now 为基准）。
    Countdown,
}

/// 单个服务的配额重置配置（持久化单元）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceEntry {
    pub id: Uuid,
    pub service_name: String,
    /// 重置间隔（小时），>0，允许浮点（如 5.5 = 5h30m）。
    pub interval_hours: f64,
    /// 上次重置时间（内部一律 UTC 存储）。
    pub last_reset_time: DateTime<Utc>,
    /// 首次提醒阈值（分钟），距重置不足该时长时触发 Condition A。
    pub reminder_threshold_minutes: u32,
    /// 周期性提醒间隔（分钟），Condition B。
    pub secondary_reminder_interval_minutes: u32,
    /// 静音：仅保留首次提醒（Condition A），抑制 Condition B。
    pub is_muted: bool,
    /// 表单输入模式（仅影响输入语义，不影响 next_reset_at 计算）。
    pub reset_config_mode: ResetConfigMode,
    /// 可选的服务主页 URL：卡片标题点击时在系统浏览器打开。旧配置无此字段时为 None。
    #[serde(default)]
    pub url: Option<String>,
}

/// 解析重置间隔表达式（单位：小时）：支持十进制数（含小数点）与
/// `+ - * /` 四则运算、括号及空白，如 `"5"`、`"5.5"`、`"5.00 * 7"`（= 35）。
/// 非法表达式、除零（结果非有限值）返回 None。
pub fn parse_interval_expr(input: &str) -> Option<f64> {
    struct Parser {
        chars: Vec<char>,
        pos: usize,
    }
    impl Parser {
        fn skip_ws(&mut self) {
            while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
        }
        fn peek(&self) -> Option<char> {
            self.chars.get(self.pos).copied()
        }
        fn next(&mut self) -> Option<char> {
            let c = self.peek();
            if c.is_some() {
                self.pos += 1;
            }
            c
        }
        // number := 数字序列（最多一个小数点）
        fn number(&mut self) -> Option<f64> {
            let start = self.pos;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() || c == '.' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == start {
                return None;
            }
            let s: String = self.chars[start..self.pos].iter().collect();
            if s.matches('.').count() > 1 {
                return None;
            }
            s.parse::<f64>().ok()
        }
        // factor := number | '(' expr ')' | '+' factor | '-' factor
        fn factor(&mut self) -> Option<f64> {
            self.skip_ws();
            match self.peek()? {
                '(' => {
                    self.next()?;
                    let v = self.expr()?;
                    self.skip_ws();
                    if self.next()? != ')' {
                        return None;
                    }
                    Some(v)
                }
                '+' => {
                    self.next()?;
                    self.factor()
                }
                '-' => {
                    self.next()?;
                    Some(-self.factor()?)
                }
                c if c.is_ascii_digit() || c == '.' => self.number(),
                _ => None,
            }
        }
        // term := factor ('*'|'/' factor)*
        fn term(&mut self) -> Option<f64> {
            let mut v = self.factor()?;
            loop {
                self.skip_ws();
                match self.peek() {
                    Some('*') => {
                        self.next()?;
                        v *= self.factor()?;
                    }
                    Some('/') => {
                        self.next()?;
                        v /= self.factor()?;
                    }
                    _ => break,
                }
            }
            Some(v)
        }
        // expr := term ('+'|'-' term)*
        fn expr(&mut self) -> Option<f64> {
            let mut v = self.term()?;
            loop {
                self.skip_ws();
                match self.peek() {
                    Some('+') => {
                        self.next()?;
                        v += self.term()?;
                    }
                    Some('-') => {
                        self.next()?;
                        v -= self.term()?;
                    }
                    _ => break,
                }
            }
            Some(v)
        }
    }
    let mut p = Parser {
        chars: input.chars().collect(),
        pos: 0,
    };
    let v = p.expr()?;
    p.skip_ws();
    if p.pos != p.chars.len() || !v.is_finite() {
        return None;
    }
    Some(v)
}

impl ServiceEntry {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.interval_hours.is_finite() || self.interval_hours <= 0.0 {
            anyhow::bail!("interval_hours 必须大于 0");
        }
        if self.reminder_threshold_minutes as f64 * 60.0 > self.interval_hours * 3600.0 {
            anyhow::bail!("提醒阈值不能超过重置间隔");
        }
        if self.secondary_reminder_interval_minutes == 0 {
            anyhow::bail!("周期性提醒间隔必须大于 0");
        }
        if self.service_name.trim().is_empty() {
            anyhow::bail!("服务名不能为空");
        }
        if let Some(url) = &self.url {
            let url = url.trim();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                anyhow::bail!("URL 必须以 http:// 或 https:// 开头");
            }
        }
        Ok(())
    }

    /// 下次重置时间 = last_reset_time + interval_hours*3600（浮点取整）。
    pub fn next_reset_at(&self) -> DateTime<Utc> {
        let secs = (self.interval_hours * 3600.0).round() as i64;
        self.last_reset_time + chrono::Duration::seconds(secs)
    }

    /// 距下次重置的剩余时长（now 之后为正；已过期则为负）。
    pub fn time_until_reset(&self, now: DateTime<Utc>) -> chrono::Duration {
        self.next_reset_at() - now
    }
}

/// 每条服务的运行时标志位（不持久化，重启后重算）。
#[derive(Debug, Clone, Default)]
pub struct RuntimeFlags {
    pub first_alert_sent: bool,
    pub last_alert_at: Option<DateTime<Utc>>,
}

/// 持久化结构：仅序列化服务列表 + 全局开关。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    pub services: Vec<ServiceEntry>,
    pub global_notifications_enabled: bool,
}

/// 全局状态变更事件：`mutate` 每次成功变更后发射，供 UI 监听刷新。
#[derive(Debug, Clone)]
pub struct AppStateChanged;

impl gpui_kit::EventEmitter<AppStateChanged> for AppState {}

/// 全局应用状态（GPUI Entity 持有）。
/// 运行时字段（runtime）与持久化字段（services / global_notifications_enabled）分离。
#[derive(Debug, Default, Clone)]
pub struct AppState {
    pub services: Vec<ServiceEntry>,
    pub runtime: HashMap<Uuid, RuntimeFlags>,
    pub global_notifications_enabled: bool,
}

impl AppState {
    pub fn from_persisted(p: PersistedState) -> Self {
        AppState {
            services: p.services,
            runtime: HashMap::new(),
            global_notifications_enabled: p.global_notifications_enabled,
        }
    }

    pub fn to_persisted(&self) -> PersistedState {
        PersistedState {
            services: self.services.clone(),
            global_notifications_enabled: self.global_notifications_enabled,
        }
    }

    pub fn add(&mut self, entry: ServiceEntry) -> anyhow::Result<()> {
        entry.validate()?;
        if self.services.iter().any(|s| s.id == entry.id) {
            anyhow::bail!("服务 id 已存在: {}", entry.id);
        }
        self.runtime.insert(entry.id, RuntimeFlags::default());
        self.services.push(entry);
        Ok(())
    }

    /// 原地更新条目；关键计时字段变更时重置运行时标志位。
    pub fn update(&mut self, updated: ServiceEntry) -> anyhow::Result<()> {
        updated.validate()?;
        let idx = self
            .services
            .iter()
            .position(|s| s.id == updated.id)
            .ok_or_else(|| anyhow::anyhow!("服务不存在: {}", updated.id))?;
        let old = &self.services[idx];
        let timing_changed = old.interval_hours != updated.interval_hours
            || old.last_reset_time != updated.last_reset_time;
        if timing_changed {
            self.runtime.insert(updated.id, RuntimeFlags::default());
        }
        self.services[idx] = updated;
        Ok(())
    }

    pub fn remove(&mut self, id: Uuid) -> bool {
        let before = self.services.len();
        self.services.retain(|s| s.id != id);
        self.runtime.remove(&id);
        self.services.len() != before
    }

    pub fn toggle_mute(&mut self, id: Uuid) -> anyhow::Result<()> {
        let entry = self
            .services
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow::anyhow!("服务不存在: {}", id))?;
        entry.is_muted = !entry.is_muted;
        Ok(())
    }

    /// 运行时标志位（不存在则返回默认值）。
    pub fn flags(&self, id: Uuid) -> RuntimeFlags {
        self.runtime.get(&id).cloned().unwrap_or_default()
    }
}

/// 将剩余时长格式化为 HH:MM:SS（不足 1 小时保持两位小时位）。
pub fn format_countdown(d: chrono::Duration) -> String {
    let s = d.num_seconds().max(0);
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    format!("{:02}:{:02}:{:02}", h, m, sec)
}

// ---------------------------------------------------------------------------
// GPUI Entity 持有与变更封装（Decision 2）
// ---------------------------------------------------------------------------

/// 在任意 `AppContext`（App / Context<View> 等）上创建 `Entity<AppState>`。
pub fn new_app_state<C: gpui_kit::AppContext>(
    cx: &mut C,
    persisted: PersistedState,
) -> gpui_kit::Entity<AppState> {
    cx.new(|_| AppState::from_persisted(persisted))
}

/// 变更封装：应用 `f` 修改状态 → `cx.notify()` 触发重渲染 → 异步持久化。
/// 持久化/变更错误通过 `on_error` 回调（如 Toast/日志）报告，不阻塞变更本身。
pub fn mutate<C, F, R>(
    state: &gpui_kit::Entity<AppState>,
    cx: &mut C,
    f: F,
    mut on_error: impl FnMut(String, &mut C) + 'static,
) -> Option<R>
where
    C: gpui_kit::AppContext,
    F: FnOnce(&mut AppState, &mut gpui_kit::Context<AppState>) -> anyhow::Result<R>,
{
    let mut result = None;
    let mut err = None;
    state.update(cx, |app_state, s_cx| match f(app_state, s_cx) {
        Ok(r) => {
            s_cx.emit(AppStateChanged);
            s_cx.notify();
            let snapshot = app_state.to_persisted();
            s_cx.spawn(async move |_weak, _async_cx| {
                if let Err(e) = crate::persistence::save(&snapshot) {
                    eprintln!("持久化失败: {:#}", e);
                }
            })
            .detach();
            result = Some(r);
        }
        Err(e) => err = Some(format!("{:#}", e)),
    });
    if let Some(e) = err {
        on_error(e, cx);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn base_entry() -> ServiceEntry {
        ServiceEntry {
            id: Uuid::new_v4(),
            service_name: "test".into(),
            interval_hours: 5.0,
            last_reset_time: Utc.with_ymd_and_hms(2026, 10, 16, 0, 0, 0).unwrap(),
            reminder_threshold_minutes: 60,
            secondary_reminder_interval_minutes: 5,
            is_muted: false,
            reset_config_mode: Default::default(),
            url: None,
        }
    }

    #[test]
    fn parse_interval_expr_cases() {
        assert_eq!(parse_interval_expr("5"), Some(5.0));
        assert_eq!(parse_interval_expr("5.5"), Some(5.5));
        assert_eq!(parse_interval_expr("5.00 * 7"), Some(35.0));
        assert_eq!(parse_interval_expr(" 12 / 4 "), Some(3.0));
        assert_eq!(parse_interval_expr("(1 + 2) * 3"), Some(9.0));
        assert_eq!(parse_interval_expr("2.5*4"), Some(10.0));
        assert_eq!(parse_interval_expr("10 - 4 + 2"), Some(8.0));
        assert_eq!(parse_interval_expr("1 / 0"), None); // 除零 → inf
        assert_eq!(parse_interval_expr("5 ** 2"), None);
        assert_eq!(parse_interval_expr("1.2.3"), None);
        assert_eq!(parse_interval_expr(""), None);
        assert_eq!(parse_interval_expr("5 h"), None);
    }

    #[test]
    fn next_reset_at_whole_hours() {
        let e = base_entry();
        assert_eq!(
            e.next_reset_at(),
            Utc.with_ymd_and_hms(2026, 10, 16, 5, 0, 0).unwrap()
        );
    }

    #[test]
    fn next_reset_at_float_hours() {
        let mut e = base_entry();
        e.interval_hours = 5.5;
        assert_eq!(
            e.next_reset_at(),
            Utc.with_ymd_and_hms(2026, 10, 16, 5, 30, 0).unwrap()
        );
    }

    #[test]
    fn next_reset_at_crosses_day() {
        let mut e = base_entry();
        e.last_reset_time = Utc.with_ymd_and_hms(2026, 10, 16, 20, 0, 0).unwrap();
        e.interval_hours = 8.0;
        assert_eq!(
            e.next_reset_at(),
            Utc.with_ymd_and_hms(2026, 10, 17, 4, 0, 0).unwrap()
        );
    }

    #[test]
    fn time_until_reset_positive_and_expired() {
        let e = base_entry();
        let now = Utc.with_ymd_and_hms(2026, 10, 16, 3, 0, 0).unwrap();
        assert_eq!(e.time_until_reset(now).num_seconds(), 7200);
        let after = Utc.with_ymd_and_hms(2026, 10, 16, 6, 0, 0).unwrap();
        assert!(e.time_until_reset(after).num_seconds() < 0);
    }

    #[test]
    fn countdown_format() {
        use chrono::Duration;
        assert_eq!(format_countdown(Duration::seconds(3661)), "01:01:01");
        assert_eq!(format_countdown(Duration::seconds(754)), "00:12:34");
        assert_eq!(format_countdown(Duration::seconds(0)), "00:00:00");
        assert_eq!(format_countdown(Duration::seconds(-10)), "00:00:00");
    }

    #[test]
    fn old_config_with_timezone_field_deserializes() {
        // 旧配置含 timezone 字段：serde 默认忽略未知字段，反序列化不报错
        let json = r#"{
            "services": [],
            "global_notifications_enabled": true,
            "timezone": "Asia/Shanghai"
        }"#;
        let state: PersistedState = serde_json::from_str(json).unwrap();
        assert!(state.services.is_empty());
        assert!(state.global_notifications_enabled);
    }

    #[test]
    fn validate_rejects_bad_input() {
        let mut e = base_entry();
        assert!(e.validate().is_ok());

        e.interval_hours = 0.0;
        assert!(e.validate().is_err());

        e.interval_hours = 0.5; // 30 分钟间隔
        e.reminder_threshold_minutes = 60; // 阈值 60 分钟超过间隔
        assert!(e.validate().is_err());

        e.reminder_threshold_minutes = 30;
        e.secondary_reminder_interval_minutes = 0;
        assert!(e.validate().is_err());

        e.secondary_reminder_interval_minutes = 5;
        e.service_name = "  ".into();
        assert!(e.validate().is_err());
    }

    #[test]
    fn add_update_remove_and_flags() {
        let mut st = AppState::default();
        let e = base_entry();
        let id = e.id;
        assert!(st.add(e.clone()).is_ok());
        assert!(st.add(e.clone()).is_err()); // id 重复

        let mut e2 = e.clone();
        e2.interval_hours = 1.0;
        assert!(st.update(e2).is_ok());
        // 计时字段变更 → flags 重置
        assert!(!st.flags(id).first_alert_sent);

        assert!(st.toggle_mute(id).is_ok());
        assert!(st.services[0].is_muted);

        assert!(st.remove(id));
        assert!(!st.remove(id));
    }
}
