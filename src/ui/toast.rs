use gpui_kit::base::animation::EffectTransition;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::{Anchor, AnyElement, App, Div, FontWeight, SharedString, Window, div, px};
use std::time::Duration;

/// 应用内提示的显示状态（序号用于避免旧定时器清除新提示；
/// exiting 标记进入淡出阶段，动画播完后才真正移除）。
#[derive(Clone)]
pub(crate) struct ToastState {
    pub seq: u64,
    pub message: SharedString,
    pub exiting: bool,
    /// 是否附加“立即升级”操作按钮（设置页检查更新发现新版本时）。
    pub action_upgrade: bool,
    /// 是否附加“立即重启 / 稍后”操作按钮（升级成功后）。
    pub action_restart: bool,
    /// 升级进行中：附加“取消”按钮，允许中断下载。
    pub action_cancel: bool,
    /// 持久提示（如升级进度）：不触发自动淡出，由展示方自行替换或结束。
    pub persistent: bool,
}

const TOAST_ANIM_MS: u64 = 180;

/// 首页与设置页共用的提示浮层：全屏（可选）遮罩 + 窗口正中横幅，
/// 进入时淡入上浮，退出时淡出下沉；遮罩阻断下层元素的鼠标交互。
/// `action` 为可选的操作按钮（如"立即升级"），渲染在横幅下方。
/// `scrim` 为 true 时绘制半透明黑蒙板压暗背景（主窗体用），
/// false 时透明浮层（设置页用，避免蒙板与面板底色混淆）。
pub(crate) fn toast_overlay(
    state: &ToastState,
    action: Option<AnyElement>,
    scrim: bool,
    cx: &App,
) -> AnyElement {
    let dur = Duration::from_millis(TOAST_ANIM_MS);
    let mut banner = toast_card(state.message.clone(), cx);
    if let Some(action) = action {
        banner = banner.child(div().mt_3().flex().justify_center().child(action));
    }
    let banner = if state.exiting {
        EffectTransition::new(dur)
            .slide_y(px(0.), px(10.))
            .apply(banner, "toast-banner-out")
            .into_any_element()
    } else {
        EffectTransition::new(dur)
            .slide_y(px(10.), px(0.))
            .apply(banner, "toast-banner-in")
            .into_any_element()
    };
    let overlay = div()
        .id("toast-scrim")
        .absolute()
        .left_0()
        .right_0()
        .top_0()
        .bottom_0()
        .when(scrim, |el| el.bg(toast_scrim()))
        .flex()
        .items_center()
        .justify_center()
        .px(px(24.))
        // 遮罩阻断下层元素的鼠标交互，提示显示期间不允许操作按钮
        .occlude()
        .on_click(|_: &gpui_kit::ClickEvent, _, _| {})
        .child(banner);
    if state.exiting {
        EffectTransition::new(dur)
            .fade(1.0, 0.0)
            .apply(overlay, "toast-overlay-out")
    } else {
        EffectTransition::new(dur)
            .fade(0.0, 1.0)
            .apply(overlay, "toast-overlay-in")
    }
    .into_any_element()
}

/// 提示浮层的半透明遮罩色：压暗背景、突出提示内容（主窗体使用；
/// 设置页底色较深不使用，避免与面板背景混淆）。
fn toast_scrim() -> gpui_kit::Hsla {
    gpui_kit::hsla(0., 0., 0., 0.35)
}

/// 首页与设置页共用的提示横幅：popover 表面色 + 主题边框 + 较强阴影，
/// 与底层窗体（服务卡片浅灰、设置面板底色）明确区分；16px 横向内边距、
/// 中等间距文案行，铺满可用宽度。
pub(crate) fn toast_card(message: SharedString, cx: &App) -> Div {
    let theme = cx.theme();
    // 支持多行文案（如“当前版本 / 最新版本”分行的更新提示），逐行渲染
    let lines: Vec<AnyElement> = message
        .split('\n')
        .map(|line| div().child(line.to_string()).into_any_element())
        .collect();
    div()
        .w_full()
        .px_4()
        .py(px(14.))
        .rounded_lg()
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border)
        .text_color(theme.popover_foreground)
        .text_center()
        .text_size(px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .shadow_lg()
        .children(lines)
}

/// 全局统一的 Toast 提示：固定在窗口底部水平居中。
/// Notification 层位于 Dialog 之上，因此在弹窗打开时提示仍显示在最上层。
pub(crate) trait BottomToast {
    fn push_toast(&mut self, note: impl Into<Notification>, cx: &mut App);
}

impl BottomToast for Window {
    fn push_toast(&mut self, note: impl Into<Notification>, cx: &mut App) {
        let note = note.into().placement(Anchor::BottomCenter);
        gpui_kit::component::WindowExt::push_notification(self, note, cx);
    }
}
