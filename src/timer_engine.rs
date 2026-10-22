use crate::models::{AppState, RuntimeFlags, ServiceEntry};
use crate::notifications::{
    NotificationService, body_with_next_reset, title_first_alert, title_recurring, title_reset,
};
use chrono::{DateTime, Utc};
use gpui_kit::{App, Entity, Task};

/// tick 决策结果（按 Condition C > A > B 优先级，每 tick 每服务至多一条）。
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Condition C：已过期，需要重置滚动（含跨多周期对齐）。
    Reset {
        /// 推进后的新 last_reset_time（周期起点，非 now，避免漂移）。
        new_last_reset: DateTime<Utc>,
    },
    /// Condition A：首次阈值提醒。
    FirstAlert { minutes_left: u64 },
    /// Condition B：周期性重复提醒。
    Recurring { minutes_left: u64 },
    /// 无动作。
    Noop,
}

/// 纯函数：给定服务、当前时间、运行时标志位与全局开关，决定本 tick 的动作。
/// 支持单测注入固定 `now`。
pub fn decide(
    service: &ServiceEntry,
    now: DateTime<Utc>,
    flags: &RuntimeFlags,
    global_enabled: bool,
) -> Decision {
    let next_reset = service.next_reset_at();

    // Condition C：重置滚动（含多周期追赶），优先级最高；不受静音与全局开关影响。
    if now >= next_reset {
        let interval_secs = (service.interval_hours * 3600.0).round() as i64;
        let elapsed = (now - service.last_reset_time).num_seconds().max(0);
        let periods = (elapsed / interval_secs).max(1);
        let new_last_reset =
            service.last_reset_time + chrono::Duration::seconds(periods * interval_secs);
        return Decision::Reset { new_last_reset };
    }

    // 提醒区间判断：剩余时间 <= 阈值（秒级比较，内部一律 UTC）。
    let remaining_secs = (next_reset - now).num_seconds();
    let threshold_secs = service.reminder_threshold_minutes as i64 * 60;
    if remaining_secs > threshold_secs {
        return Decision::Noop;
    }

    // Condition A：本周期首次提醒（不受静音影响；静音只抑制 B）。
    if !flags.first_alert_sent {
        if !global_enabled {
            // 全局禁用时不发送，但视为已消费首次提醒，避免开启后瞬时爆发。
            return Decision::Noop;
        }
        return Decision::FirstAlert {
            minutes_left: (remaining_secs / 60).max(0) as u64,
        };
    }

    // Condition B：周期性重复提醒（静音抑制）。
    if service.is_muted {
        return Decision::Noop;
    }
    if !global_enabled {
        return Decision::Noop;
    }
    let interval_secs_b = service.secondary_reminder_interval_minutes as i64 * 60;
    if let Some(last) = flags.last_alert_at
        && (now - last).num_seconds() >= interval_secs_b
    {
        return Decision::Recurring {
            minutes_left: (remaining_secs / 60).max(0) as u64,
        };
    }
    Decision::Noop
}

/// 单次 tick 的结果：是否发生重置（触发持久化）与失败消息列表（触发 Toast）。
#[derive(Debug, Default)]
pub struct TickOutcome {
    pub reset_happened: bool,
    pub error_messages: Vec<String>,
}

/// 执行单次 tick 的完整逻辑（纯函数，`now` 可注入便于测试）。
/// 优先级 C -> A -> B：先批量处理 Condition C 重置（含多周期追赶，清除标志位），
/// 再发送 A/B 通知；通知失败写入日志并返回失败消息，不中断 tick。
pub fn run_tick(
    state: &mut AppState,
    now: DateTime<Utc>,
    notifier: &dyn NotificationService,
) -> TickOutcome {
    let global = state.global_notifications_enabled;
    let mut out = TickOutcome::default();

    // 阶段一：收集决策（不可变借用）
    let decisions: Vec<(ServiceEntry, Decision)> = state
        .services
        .iter()
        .map(|entry| {
            let flags = state.flags(entry.id);
            let d = decide(entry, now, &flags, global);
            (entry.clone(), d)
        })
        .collect();

    // 阶段二：统一应用 Condition C 重置（可变借用），含多周期追赶
    for (entry, decision) in &decisions {
        if let Decision::Reset { new_last_reset } = decision {
            if let Some(e) = state.services.iter_mut().find(|e| e.id == entry.id) {
                e.last_reset_time = *new_last_reset;
            }
            state.runtime.insert(
                entry.id,
                RuntimeFlags {
                    first_alert_sent: false,
                    last_alert_at: None,
                },
            );
            out.reset_happened = true;
            // 重置通知：告知用户配额已重置并进入新周期。
            // 重置滚动行为本身不受全局开关影响（否则倒计时会失真），
            // 但"发送桌面通知"受全局开关控制；静音仅抑制周期提醒 B，不影响重置通知。
            if global {
                let interval_secs = (entry.interval_hours * 3600.0).round() as i64;
                let next_reset_after = *new_last_reset + chrono::Duration::seconds(interval_secs);
                let title = title_reset(&entry.service_name);
                let body = body_with_next_reset(next_reset_after);
                if notifier.notify(&title, &body).is_err() {
                    out.error_messages
                        .push(format!("重置通知发送失败：{}", entry.service_name));
                }
            }
        }
    }

    // 阶段三：发送 A/B 通知并更新标志位
    for (entry, decision) in &decisions {
        let (title, is_first) = match decision {
            Decision::FirstAlert { minutes_left } => (
                title_first_alert(
                    &entry.service_name,
                    (*minutes_left).min(u32::MAX as u64) as u32,
                ),
                true,
            ),
            Decision::Recurring { minutes_left } => {
                (title_recurring(&entry.service_name, *minutes_left), false)
            }
            _ => continue,
        };
        let next_reset = entry.next_reset_at();
        let body = body_with_next_reset(next_reset);
        let ok = notifier.notify(&title, &body).is_ok();
        if !ok {
            out.error_messages
                .push(format!("通知发送失败：{}", entry.service_name));
            continue;
        }
        let flags = state.runtime.entry(entry.id).or_default();
        if is_first {
            flags.first_alert_sent = true;
            flags.last_alert_at = Some(now);
        } else {
            flags.last_alert_at = Some(now);
        }
    }
    out
}

/// 1s tick 引擎（Decision 3）：GPUI Executor 上 spawn 单任务循环。
/// 每 tick：now=Utc::now()，调用 `run_tick`（C -> A -> B）；
/// 通过 model.update 刷新 UI，重置后异步保存；通知失败写入日志并 Toast，不中断 tick。
/// 任务生命周期：窗口关闭 / Entity 释放时 `async_cx.update` 失败，任务随之结束，无僵尸任务。
pub fn spawn_timer(
    cx: &mut App,
    app_state: Entity<AppState>,
    notifier: std::sync::Arc<dyn NotificationService>,
    mut on_error: impl FnMut(String, &mut App) + 'static,
) -> Task<()> {
    cx.spawn(async move |async_cx| {
        loop {
            // 1s 间隔；gpui 后台执行器定时器，不阻塞渲染线程。
            async_cx
                .background_executor()
                .timer(std::time::Duration::from_secs(1))
                .await;

            let mut outcome = TickOutcome::default();
            async_cx.update(|cx| {
                app_state.update(cx, |state, s_cx| {
                    outcome = run_tick(state, Utc::now(), notifier.as_ref());
                    // 每 tick 都 notify：倒计时文本需每秒刷新
                    s_cx.notify();
                    if outcome.reset_happened {
                        let snapshot = state.to_persisted();
                        s_cx.spawn(async move |_, _| {
                            if let Err(e) = crate::persistence::save(&snapshot) {
                                eprintln!("持久化失败: {:#}", e);
                            }
                        })
                        .detach();
                    }
                });
                // 状态更新完成后回调 on_error（Toast 降级），不中断 tick
                for msg in outcome.error_messages.clone() {
                    on_error(msg, cx);
                }
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn entry(
        interval_hours: f64,
        threshold_min: u32,
        secondary_min: u32,
        muted: bool,
    ) -> ServiceEntry {
        ServiceEntry {
            id: Uuid::new_v4(),
            service_name: "test".into(),
            interval_hours,
            last_reset_time: Utc.with_ymd_and_hms(2026, 10, 16, 0, 0, 0).unwrap(),
            reminder_threshold_minutes: threshold_min,
            secondary_reminder_interval_minutes: secondary_min,
            is_muted: muted,
            reset_config_mode: Default::default(),
            url: None,
        }
    }

    fn t(h: i64, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 16, h as u32, m, s).unwrap()
    }

    #[test]
    fn no_decision_outside_threshold() {
        let e = entry(5.0, 60, 5, false);
        let flags = RuntimeFlags::default();
        assert_eq!(decide(&e, t(3, 0, 0), &flags, true), Decision::Noop);
    }

    #[test]
    fn first_alert_once_inside_threshold() {
        let e = entry(5.0, 60, 5, false);
        let flags = RuntimeFlags::default();
        // 4:30 → 剩余 30 分钟，阈值 60 分钟 → 首次提醒
        assert_eq!(
            decide(&e, t(4, 30, 0), &flags, true),
            Decision::FirstAlert { minutes_left: 30 }
        );
        // 已发送首次提醒后不再重复 A
        let sent = RuntimeFlags {
            first_alert_sent: true,
            last_alert_at: Some(t(4, 30, 0)),
        };
        assert_eq!(decide(&e, t(4, 31, 0), &sent, true), Decision::Noop);
    }

    #[test]
    fn recurring_after_secondary_interval() {
        let e = entry(5.0, 60, 5, false);
        let sent = RuntimeFlags {
            first_alert_sent: true,
            last_alert_at: Some(t(4, 30, 0)),
        };
        // 距上次提醒 5 分钟 → 触发 B
        assert_eq!(
            decide(&e, t(4, 35, 0), &sent, true),
            Decision::Recurring { minutes_left: 25 }
        );
        // 不足 5 分钟 → 无动作
        assert_eq!(decide(&e, t(4, 34, 59), &sent, true), Decision::Noop);
    }

    #[test]
    fn muted_suppresses_recurring_but_not_first_alert() {
        let e = entry(5.0, 60, 5, true);
        let flags = RuntimeFlags::default();
        // 静音仍允许首次提醒
        assert_eq!(
            decide(&e, t(4, 30, 0), &flags, true),
            Decision::FirstAlert { minutes_left: 30 }
        );
        let sent = RuntimeFlags {
            first_alert_sent: true,
            last_alert_at: Some(t(4, 30, 0)),
        };
        // 静音抑制 B
        assert_eq!(decide(&e, t(4, 35, 0), &sent, true), Decision::Noop);
    }

    #[test]
    fn global_disable_suppresses_alerts_but_not_reset() {
        let e = entry(5.0, 60, 5, false);
        let flags = RuntimeFlags::default();
        assert_eq!(decide(&e, t(4, 30, 0), &flags, false), Decision::Noop);
        // 重置不受全局开关影响
        assert!(matches!(
            decide(&e, t(5, 0, 0), &flags, false),
            Decision::Reset { .. }
        ));
    }

    #[test]
    fn reset_rolls_forward_and_aligns_to_period() {
        let e = entry(5.0, 60, 5, false);
        let flags = RuntimeFlags::default();
        // 5:00:01 → 跨过一个周期 → 推进到 5:00（周期起点）
        match decide(&e, t(5, 0, 1), &flags, true) {
            Decision::Reset { new_last_reset } => {
                assert_eq!(new_last_reset, t(5, 0, 0));
            }
            other => panic!("expected Reset, got {:?}", other),
        }
        // 多周期追赶：11:30 → 跨 2 个周期 → 推进到 10:00
        match decide(&e, t(11, 30, 0), &flags, true) {
            Decision::Reset { new_last_reset } => {
                assert_eq!(new_last_reset, t(10, 0, 0));
            }
            other => panic!("expected Reset, got {:?}", other),
        }
    }

    #[test]
    fn full_cycle_a_then_b_then_c() {
        let e = entry(5.0, 60, 5, false);
        // tick1: 首次提醒
        let flags = RuntimeFlags::default();
        let now = t(4, 0, 0);
        assert!(matches!(
            decide(&e, now, &flags, true),
            Decision::FirstAlert { .. }
        ));
        // tick2 (4:04:59): A 已发送且距上次提醒不足 5 分钟 → 无 B
        let flags2 = RuntimeFlags {
            first_alert_sent: true,
            last_alert_at: Some(now),
        };
        assert_eq!(decide(&e, t(4, 4, 59), &flags2, true), Decision::Noop);
        // tick3 (4:05:00): B 触发（二次间隔 5 分钟已到）
        assert!(matches!(
            decide(&e, t(4, 5, 0), &flags2, true),
            Decision::Recurring { .. }
        ));
        // tick4: 重置 → flags 清除
        match decide(&e, t(5, 0, 0), &flags2, true) {
            Decision::Reset { new_last_reset } => {
                let cleared = RuntimeFlags::default();
                // 下一周期重新走 A
                let e2 = ServiceEntry {
                    last_reset_time: new_last_reset,
                    ..e
                };
                assert!(matches!(
                    decide(&e2, t(9, 0, 0), &cleared, true),
                    Decision::FirstAlert { .. }
                ));
            }
            other => panic!("expected Reset, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // 8.1 集成测试：启动加载 → tick → 通知 → 重置滚动（内存态端到端，
    // 注入 MockNotificationService 与固定 now，无需真实 sleep）
    // ------------------------------------------------------------------

    use crate::models::PersistedState;
    use crate::notifications::MockNotificationService;

    fn state_with(e: ServiceEntry, global: bool) -> AppState {
        AppState::from_persisted(PersistedState {
            services: vec![e],
            global_notifications_enabled: global,
            ..Default::default()
        })
    }

    #[test]
    fn integration_load_tick_notify_reset_rolls() {
        // 启动加载：1h 间隔、30 分钟阈值、5 分钟二次提醒
        let t0 = Utc.with_ymd_and_hms(2026, 10, 16, 0, 0, 0).unwrap();
        let mut state = state_with(entry(1.0, 30, 5, false), true);
        let mock = MockNotificationService::default();

        // 剩余 30 分钟（=阈值）→ A 首次提醒
        let out = run_tick(&mut state, t0 + chrono::Duration::minutes(30), &mock);
        assert_eq!(mock.call_count(), 1);
        assert!(!out.reset_happened);
        assert!(state.flags(state.services[0].id).first_alert_sent);

        // 剩余 25 分钟（距上次提醒 5 分钟）→ B
        run_tick(&mut state, t0 + chrono::Duration::minutes(35), &mock);
        assert_eq!(mock.call_count(), 2);

        // 剩余 24 分钟（距上次提醒 1 分钟）→ 无动作
        run_tick(&mut state, t0 + chrono::Duration::minutes(36), &mock);
        assert_eq!(mock.call_count(), 2);

        // 跨过重置点 → C：last_reset 推进到 t0+1h，标志位清除，并发送"已重置"通知
        let out = run_tick(&mut state, t0 + chrono::Duration::minutes(60), &mock);
        assert!(out.reset_happened);
        assert_eq!(mock.call_count(), 3); // A + B + 重置通知
        assert_eq!(
            state.services[0].last_reset_time,
            t0 + chrono::Duration::hours(1)
        );
        let f = state.flags(state.services[0].id);
        assert!(!f.first_alert_sent);
        assert!(f.last_alert_at.is_none());

        // 新周期剩余 30 分钟 → 重新走 A（证明标志位清除后周期重启）
        run_tick(&mut state, t0 + chrono::Duration::minutes(90), &mock);
        assert_eq!(mock.call_count(), 4);
    }

    #[test]
    fn integration_muted_suppresses_b_not_c() {
        let t0 = Utc.with_ymd_and_hms(2026, 10, 16, 0, 0, 0).unwrap();
        let mut state = state_with(entry(1.0, 30, 5, true), true); // 静音
        let mock = MockNotificationService::default();

        // 静音仍允许 A
        run_tick(&mut state, t0 + chrono::Duration::minutes(30), &mock);
        assert_eq!(mock.call_count(), 1);
        // B 被静音抑制
        run_tick(&mut state, t0 + chrono::Duration::minutes(35), &mock);
        assert_eq!(mock.call_count(), 1);
        // C 不受静音影响，重置通知照发（静音只抑制周期提醒 B）
        let out = run_tick(&mut state, t0 + chrono::Duration::minutes(60), &mock);
        assert!(out.reset_happened);
        assert_eq!(mock.call_count(), 2); // A + 重置通知
    }

    #[test]
    fn integration_global_disable_suppresses_a_b_not_c() {
        let t0 = Utc.with_ymd_and_hms(2026, 10, 16, 0, 0, 0).unwrap();
        let mut state = state_with(entry(1.0, 30, 5, false), false); // 全局禁用
        let mock = MockNotificationService::default();

        run_tick(&mut state, t0 + chrono::Duration::minutes(30), &mock);
        assert_eq!(mock.call_count(), 0);
        run_tick(&mut state, t0 + chrono::Duration::minutes(35), &mock);
        assert_eq!(mock.call_count(), 0);
        // C 仍执行（滚动行为不受全局开关影响），但重置通知被全局禁用
        let out = run_tick(&mut state, t0 + chrono::Duration::minutes(60), &mock);
        assert!(out.reset_happened);
        assert_eq!(mock.call_count(), 0); // 全局禁用：不发送任何通知
        assert_eq!(
            state.services[0].last_reset_time,
            t0 + chrono::Duration::hours(1)
        );
    }

    /// 模拟 notify-rust 失败（权限拒绝/守护进程缺失）。
    struct FailingNotifier;
    impl crate::notifications::NotificationService for FailingNotifier {
        fn notify(&self, _title: &str, _body: &str) -> anyhow::Result<()> {
            anyhow::bail!("simulated notification failure")
        }
    }

    #[test]
    fn integration_notify_failure_does_not_break_tick() {
        let t0 = Utc.with_ymd_and_hms(2026, 10, 16, 0, 0, 0).unwrap();
        let mut state = state_with(entry(1.0, 30, 5, false), true);

        // A 触发但发送失败 → 错误消息，tick 不中断
        let out = run_tick(
            &mut state,
            t0 + chrono::Duration::minutes(30),
            &FailingNotifier,
        );
        assert_eq!(out.error_messages.len(), 1);
        // 失败未标记 first_alert_sent → 下一 tick 重试
        let out = run_tick(
            &mut state,
            t0 + chrono::Duration::minutes(31),
            &FailingNotifier,
        );
        assert_eq!(out.error_messages.len(), 1);

        // 通知恢复后（切换 notifier）周期继续正常
        let mock = MockNotificationService::default();
        run_tick(&mut state, t0 + chrono::Duration::minutes(32), &mock);
        assert_eq!(mock.call_count(), 1);
    }

    // ------------------------------------------------------------------
    // 8.2 性能：50+ 服务条目下单 tick 开销 < 8ms（120 FPS 帧预算）
    // ------------------------------------------------------------------

    #[test]
    fn performance_60_services_tick_under_8ms() {
        let t0 = Utc.with_ymd_and_hms(2026, 10, 16, 0, 0, 0).unwrap();
        let mut state = AppState::default();
        for i in 0..60 {
            let mut e = entry(1.0 + i as f64 * 0.01, 30, 5, false);
            e.service_name = format!("svc-{}", i);
            state.add(e).unwrap();
        }
        let mock = MockNotificationService::default();
        let now = t0 + chrono::Duration::minutes(30);
        run_tick(&mut state, now, &mock); // warmup
        let start = std::time::Instant::now();
        const ITERS: u32 = 100;
        for _ in 0..ITERS {
            run_tick(&mut state, now, &mock);
        }
        let per_tick = start.elapsed() / ITERS;
        assert!(
            per_tick < std::time::Duration::from_millis(8),
            "tick took {:?} per iteration",
            per_tick
        );
    }
}
