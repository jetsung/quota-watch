use crate::models::{self, AppState, ServiceEntry};
use crate::ui::checkbox::render_checkbox;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, Entity, Focusable as _, IntoElement, ParentElement, Render,
    SharedString, Window, div, px,
};
use uuid::Uuid;

const LAST_RESET_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// Add/Edit 共用的表单视图。
/// 字段：service_name / interval_hours /（NextReset 或 Countdown 模式的重置时间输入）/
/// reminder_threshold_minutes / secondary_reminder_interval_minutes / is_muted。
/// 提交时执行 `ServiceEntry::validate`，失败显示内联错误；成功则关闭对话框并 Toast。
/// 任意输入框内按回车等同点击"保存"。编辑预填充；关键计时字段变更由 `AppState::update` 自动重置 RuntimeFlags。
pub struct AddEditView {
    state: Entity<AppState>,
    /// 自绘标题栏的标题文字（编辑服务：xxx / 添加服务）。
    title: String,
    editing: Option<ServiceEntry>,
    /// 当前表单的重置配置模式（仅影响输入语义）。
    /// 枚举仍保留 LastReset 变体以兼容旧配置序列化，但表单不再提供该选项，
    /// 打开编辑旧 LastReset 条目时自动按 NextReset 模式显示。
    reset_config_mode: models::ResetConfigMode,
    name: Entity<InputState>,
    interval: Entity<InputState>,
    /// NextReset 模式：下次重置的绝对时间（按系统本地时区解释）。
    next_reset: Entity<InputState>,
    /// 可选服务主页 URL：卡片标题点击时在系统浏览器打开。
    url: Entity<InputState>,
    /// Countdown 模式：距下次重置的剩余时长（时/分/秒）。
    cd_h: Entity<InputState>,
    cd_m: Entity<InputState>,
    cd_s: Entity<InputState>,
    threshold: Entity<InputState>,
    secondary: Entity<InputState>,
    muted: bool,
    error: Option<SharedString>,
}

fn field_label(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(13.))
        .font_weight(gpui_kit::FontWeight::BOLD)
        .mb_1()
        .child(text.into())
}

/// 按系统本地时区格式化 UTC 时间（表单预填/展示统一入口）。
fn format_dt_local(dt: DateTime<Utc>) -> String {
    dt.with_timezone(&chrono::Local)
        .format(LAST_RESET_FORMAT)
        .to_string()
}

/// 将剩余时长格式化为 "HH:MM:SS"（负值截断为 0）。
fn duration_hms(d: chrono::Duration) -> String {
    let s = d.num_seconds().max(0);
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// 拆分 "HH:MM:SS" 为时/分/秒三个字符串。
fn split_hms(s: &str) -> (String, String, String) {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        (parts[0].into(), parts[1].into(), parts[2].into())
    } else {
        ("00".into(), "00".into(), "00".into())
    }
}

impl AddEditView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        state: Entity<AppState>,
        editing: Option<ServiceEntry>,
        title: String,
    ) -> Self {
        // 预填统一按系统本地时区格式化
        let now = Utc::now();
        let (
            name_default,
            interval_default,
            next_reset_default,
            cd_default,
            threshold_default,
            secondary_default,
            muted,
        ) = match &editing {
            Some(e) => (
                e.service_name.clone(),
                format!("{:.2}", e.interval_hours),
                format_dt_local(e.next_reset_at()),
                duration_hms(e.next_reset_at() - now),
                e.reminder_threshold_minutes.to_string(),
                e.secondary_reminder_interval_minutes.to_string(),
                e.is_muted,
            ),
            None => {
                let interval_h = 5.0f64;
                let next = now + chrono::Duration::seconds((interval_h * 3600.0).round() as i64);
                (
                    String::new(),
                    "5".to_string(),
                    format_dt_local(next),
                    duration_hms(next - now),
                    "60".to_string(),
                    "5".to_string(),
                    false,
                )
            }
        };
        let (cd_h_default, cd_m_default, cd_s_default) = split_hms(&cd_default);
        let url_default = editing
            .as_ref()
            .and_then(|e| e.url.clone())
            .unwrap_or_default();

        // 旧条目可能持久化为 LastReset 模式，表单已不提供该选项，统一按 NextReset 显示
        let initial_mode = match editing.as_ref().map(|e| e.reset_config_mode) {
            Some(models::ResetConfigMode::Countdown) => models::ResetConfigMode::Countdown,
            _ => models::ResetConfigMode::NextReset,
        };

        let view = Self {
            state,
            title,
            reset_config_mode: initial_mode,
            editing,
            name: cx.new(|cx| InputState::new(window, cx).default_value(name_default)),
            interval: cx.new(|cx| InputState::new(window, cx).default_value(interval_default)),
            next_reset: cx.new(|cx| InputState::new(window, cx).default_value(next_reset_default)),
            url: cx.new(|cx| InputState::new(window, cx).default_value(url_default)),
            cd_h: cx.new(|cx| InputState::new(window, cx).default_value(cd_h_default)),
            cd_m: cx.new(|cx| InputState::new(window, cx).default_value(cd_m_default)),
            cd_s: cx.new(|cx| InputState::new(window, cx).default_value(cd_s_default)),
            threshold: cx.new(|cx| InputState::new(window, cx).default_value(threshold_default)),
            secondary: cx.new(|cx| InputState::new(window, cx).default_value(secondary_default)),
            muted,
            error: None,
        };
        // 任意输入框内按回车 → 保存提交（与"保存"按钮等效）
        let inputs: [Entity<InputState>; 9] = [
            view.name.clone(),
            view.interval.clone(),
            view.next_reset.clone(),
            view.url.clone(),
            view.cd_h.clone(),
            view.cd_m.clone(),
            view.cd_s.clone(),
            view.threshold.clone(),
            view.secondary.clone(),
        ];
        for input in inputs {
            // subscribe_in：回调携带 &mut Window，可直接提交（Toast/关闭对话框都在窗口回调栈内完成）
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit(window, cx);
                }
            })
            .detach();
        }
        view
    }

    /// 返回服务名输入框的焦点句柄（供对话框打开时设置初始焦点）。
    pub fn name_focus_handle(&self, cx: &App) -> gpui_kit::FocusHandle {
        self.name.read(cx).focus_handle(cx)
    }

    /// 将 "YYYY-MM-DD HH:MM:SS" 文本按系统本地时区解释为 UTC 时间。
    fn parse_dt_local(s: &str) -> Option<DateTime<Utc>> {
        let naive = NaiveDateTime::parse_from_str(s.trim(), LAST_RESET_FORMAT).ok()?;
        chrono::Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|dt| dt.with_timezone(&Utc))
    }

    /// 从当前模式输入推导 last_reset_time（UTC），作为模式切换时的语义锚点。
    fn current_last_reset(&self, cx: &Context<Self>) -> Option<DateTime<Utc>> {
        let now = Utc::now();
        let interval: f64 = self.interval.read(cx).value().parse().ok()?;
        match self.reset_config_mode {
            models::ResetConfigMode::NextReset => {
                let next = Self::parse_dt_local(&self.next_reset.read(cx).value())?;
                Some(next - chrono::Duration::seconds((interval * 3600.0).round() as i64))
            }
            models::ResetConfigMode::Countdown => {
                let h: u64 = self.cd_h.read(cx).value().trim().parse().ok()?;
                let m: u64 = self.cd_m.read(cx).value().trim().parse().ok()?;
                let s: u64 = self.cd_s.read(cx).value().trim().parse().ok()?;
                let next = now + chrono::Duration::seconds((h * 3600 + m * 60 + s) as i64);
                Some(next - chrono::Duration::seconds((interval * 3600.0).round() as i64))
            }
            // 表单不再提供 LastReset 模式（仅枚举保留以兼容旧配置）
            _ => None,
        }
    }

    /// 切换重置配置模式：以当前推导的 last_reset_time 为锚点重新预填各输入框。
    fn switch_mode(
        &mut self,
        mode: models::ResetConfigMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reset_config_mode == mode
            || !matches!(
                mode,
                models::ResetConfigMode::NextReset | models::ResetConfigMode::Countdown
            )
        {
            return;
        }
        let now = Utc::now();
        let interval: f64 = self
            .interval
            .read(cx)
            .value()
            .parse::<f64>()
            .unwrap_or(5.0)
            .max(0.001);
        let anchor = self.current_last_reset(cx).unwrap_or(now);
        let next = anchor + chrono::Duration::seconds((interval * 3600.0).round() as i64);
        let (cd_h, cd_m, cd_s) = split_hms(&duration_hms(next - now));
        self.next_reset =
            cx.new(|cx| InputState::new(window, cx).default_value(format_dt_local(next)));
        self.cd_h = cx.new(|cx| InputState::new(window, cx).default_value(cd_h));
        self.cd_m = cx.new(|cx| InputState::new(window, cx).default_value(cd_m));
        self.cd_s = cx.new(|cx| InputState::new(window, cx).default_value(cd_s));
        self.reset_config_mode = mode;
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 1. 读取并解析表单
        let name = self.name.read(cx).value().to_string().trim().to_string();
        let interval_text = self.interval.read(cx).value().trim().to_string();
        let interval: f64 = match models::parse_interval_expr(&interval_text) {
            Some(v) => v,
            None => {
                self.error = Some("重置间隔必须是数字或公式，如 5 * 7（小时）".into());
                cx.notify();
                return;
            }
        };
        let threshold: u32 = match self.threshold.read(cx).value().parse() {
            Ok(v) => v,
            Err(_) => {
                self.error = Some("提醒阈值必须是整数（分钟）".into());
                cx.notify();
                return;
            }
        };
        let secondary: u32 = match self.secondary.read(cx).value().parse() {
            Ok(v) => v,
            Err(_) => {
                self.error = Some("周期提醒间隔必须是整数（分钟）".into());
                cx.notify();
                return;
            }
        };
        // 两种模式统一推导 last_reset_time（UTC），保证 next_reset_at 语义一致
        let now = Utc::now();
        let last_reset: DateTime<Utc> = match self.reset_config_mode {
            models::ResetConfigMode::Countdown => {
                // 以提交时刻 now 为基准：next = now + duration，反推 last_reset
                let h: u64 = match self.cd_h.read(cx).value().trim().parse() {
                    Ok(v) => v,
                    Err(_) => {
                        self.error = Some("剩余时长（时）必须是非负整数".into());
                        cx.notify();
                        return;
                    }
                };
                let m: u64 = match self.cd_m.read(cx).value().trim().parse() {
                    Ok(v) => v,
                    Err(_) => {
                        self.error = Some("剩余时长（分）必须是非负整数".into());
                        cx.notify();
                        return;
                    }
                };
                let s: u64 = match self.cd_s.read(cx).value().trim().parse() {
                    Ok(v) => v,
                    Err(_) => {
                        self.error = Some("剩余时长（秒）必须是非负整数".into());
                        cx.notify();
                        return;
                    }
                };
                if h == 0 && m == 0 && s == 0 {
                    self.error = Some("剩余时长不能为 0".into());
                    cx.notify();
                    return;
                }
                let next = now + chrono::Duration::seconds((h * 3600 + m * 60 + s) as i64);
                next - chrono::Duration::seconds((interval * 3600.0).round() as i64)
            }
            // NextReset（及旧配置残留的 LastReset，表单已按 NextReset 回退显示）
            _ => {
                // 按系统本地时区解释输入的绝对时间，反推 last_reset = next - interval
                match Self::parse_dt_local(&self.next_reset.read(cx).value()) {
                    Some(next) => {
                        next - chrono::Duration::seconds((interval * 3600.0).round() as i64)
                    }
                    None => {
                        self.error = Some("下次重置时间格式应为 YYYY-MM-DD HH:MM:SS".into());
                        cx.notify();
                        return;
                    }
                }
            }
        };

        // URL：空输入视为未设置（可选字段）
        let url_input = self.url.read(cx).value().trim().to_string();
        let url = if url_input.is_empty() {
            None
        } else {
            Some(url_input)
        };

        // 2. 构造条目并写入状态（校验失败 → 内联错误）
        let state = self.state.clone();
        let is_edit = self.editing.is_some();
        let id = self
            .editing
            .as_ref()
            .map(|e| e.id)
            .unwrap_or_else(Uuid::new_v4);
        let entry = ServiceEntry {
            id,
            service_name: name,
            interval_hours: interval,
            last_reset_time: last_reset,
            reminder_threshold_minutes: threshold,
            secondary_reminder_interval_minutes: secondary,
            is_muted: self.muted,
            reset_config_mode: self.reset_config_mode,
            url,
        };
        let handle = window.window_handle();
        let result = models::mutate(
            &state,
            cx,
            move |app_state, _| {
                if is_edit {
                    app_state.update(entry.clone())
                } else {
                    app_state.add(entry.clone())
                }
            },
            move |msg, app| {
                handle
                    .update(app, |_, win, app| {
                        win.push_notification(Notification::error(msg), app);
                    })
                    .ok();
            },
        );

        if result.is_some() {
            let msg = if is_edit {
                "已更新服务"
            } else {
                "已添加服务"
            };
            window.push_notification(Notification::success(msg), cx);
            window.close_all_dialogs(cx);
        }
    }
}

impl Render for AddEditView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // 自绘标题栏：标题居左、关闭按钮✕居右（替代库 Dialog 自带的 title/X）
        let title_bar = h_flex()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .id("dialog-close-x")
                    .text_size(px(14.))
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(|d| d.text_color(theme.foreground))
                    .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                        window.close_all_dialogs(cx);
                    }))
                    .child("✕"),
            );
        v_flex()
            .w_full()
            .p_4()
            .gap_3()
            .child(title_bar)
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("服务名"))
                    .child(Input::new(&self.name).w_full()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("重置间隔（小时，支持公式，如 5 * 7）"))
                    .child(Input::new(&self.interval).w_full()),
            )
            .child(
                // 模式切换控件（NextReset / Countdown 两种重置配置模式）
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("mode-next-reset")
                            .label("下次重置时间")
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .when(
                                self.reset_config_mode == models::ResetConfigMode::NextReset,
                                |b| b.primary(),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.switch_mode(models::ResetConfigMode::NextReset, window, cx);
                            })),
                    )
                    .child(
                        Button::new("mode-countdown")
                            .label("倒计时")
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .when(
                                self.reset_config_mode == models::ResetConfigMode::Countdown,
                                |b| b.primary(),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.switch_mode(models::ResetConfigMode::Countdown, window, cx);
                            })),
                    ),
            )
            .child(match self.reset_config_mode {
                models::ResetConfigMode::NextReset => v_flex()
                    .gap_1()
                    .child(field_label("YYYY-MM-DD HH:MM:SS"))
                    .child(Input::new(&self.next_reset).w_full())
                    .into_any_element(),
                _ => v_flex()
                    .gap_1()
                    .child(field_label("时 / 分 / 秒"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&self.cd_h).flex_grow_1())
                            .child(Input::new(&self.cd_m).flex_grow_1())
                            .child(Input::new(&self.cd_s).flex_grow_1()),
                    )
                    .into_any_element(),
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("服务主页 URL（可选）"))
                    .child(Input::new(&self.url).w_full()),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_grow_1()
                            .gap_1()
                            .child(field_label("提醒阈值（分钟）"))
                            .child(Input::new(&self.threshold)),
                    )
                    .child(
                        v_flex()
                            .flex_grow_1()
                            .gap_1()
                            .child(field_label("周期提醒间隔（分钟）"))
                            .child(Input::new(&self.secondary)),
                    ),
            )
            .child(render_checkbox(
                "is-muted",
                "静音重复提醒（仅首次提醒）",
                self.muted,
                cx.listener(|this, checked: &bool, _, cx| {
                    this.muted = *checked;
                    cx.notify();
                }),
                cx,
            ))
            .when_some(self.error.clone(), |el, err| {
                el.child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(cx.theme().danger)
                        .child(err),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel")
                            .label("取消")
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .on_click(cx.listener(move |_, _, window, cx| {
                                window.close_all_dialogs(cx);
                            })),
                    )
                    .child(
                        Button::new("submit")
                            .primary()
                            .label("保存")
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.submit(window, cx);
                            })),
                    ),
            )
    }
}
