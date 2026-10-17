use anyhow::Context as _;
use chrono::{Local, Utc};
use std::sync::atomic::{AtomicUsize, Ordering};

/// 通知服务抽象：便于测试注入 Mock 捕获调用。
pub trait NotificationService: Send + Sync {
    fn notify(&self, title: &str, body: &str) -> anyhow::Result<()>;
}

/// 桌面通知实现（notify-rust）。
pub struct DesktopNotificationService;

impl NotificationService for DesktopNotificationService {
    fn notify(&self, title: &str, body: &str) -> anyhow::Result<()> {
        notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .timeout(notify_rust::Timeout::Milliseconds(8000))
            .show()
            .map(|_| ())
            .context("系统通知发送失败（检查通知权限 / 守护进程）")
    }
}

/// 测试用 Mock：记录调用次数（按序号可断言）。
#[derive(Default)]
#[allow(dead_code)] // 仅测试使用，bin 编译时静默
pub struct MockNotificationService {
    pub calls: AtomicUsize,
}

impl MockNotificationService {
    #[allow(dead_code)]
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl NotificationService for MockNotificationService {
    fn notify(&self, _title: &str, _body: &str) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// 通知文案（Decision 4）：
/// - Condition A: "[Service] 将在 X 分钟后重置"
/// - Condition B: "[Service] 即将重置！还剩 Y 分钟"
///
/// 正文附加本地时区 next_reset_at。
pub fn title_first_alert(service_name: &str, minutes: u32) -> String {
    format!("[{}] 将在 {} 分钟后重置", service_name, minutes)
}

pub fn title_recurring(service_name: &str, minutes: u64) -> String {
    format!("[{}] 即将重置！还剩 {} 分钟", service_name, minutes.max(1))
}

pub fn title_reset(service_name: &str) -> String {
    format!("[{}] 配额已重置", service_name)
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
    fn message_texts() {
        assert_eq!(
            title_first_alert("openai", 30),
            "[openai] 将在 30 分钟后重置"
        );
        assert_eq!(
            title_recurring("openai", 2),
            "[openai] 即将重置！还剩 2 分钟"
        );
        assert_eq!(title_reset("gpt"), "[gpt] 配额已重置");
    }

    #[test]
    fn body_local_time() {
        let t = Utc.with_ymd_and_hms(2026, 10, 16, 5, 0, 0).unwrap();
        let body = body_with_next_reset(t);
        assert!(body.contains("下次重置："));
        assert!(body.contains("2026-10-16"));
    }
}
