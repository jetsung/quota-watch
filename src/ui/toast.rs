use gpui_kit::base::animation::EffectTransition;
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
}

const TOAST_ANIM_MS: u64 = 180;

/// 首页与设置页共用的提示浮层：全屏遮罩 + 窗口正中横幅，
/// 进入时淡入上浮，退出时淡出下沉；遮罩阻断下层元素的鼠标交互。
pub(crate) fn toast_overlay(state: &ToastState) -> AnyElement {
    let dur = Duration::from_millis(TOAST_ANIM_MS);
    let banner = toast_card(state.message.clone());
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
        .bg(toast_scrim())
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

/// 首页提示浮层的半透明遮罩色，用于压暗背景、突出提示内容。
pub(crate) fn toast_scrim() -> gpui_kit::Hsla {
    gpui_kit::hsla(0., 0., 0., 0.35)
}

/// 首页与设置页共用的提示横幅：无边框、浅灰背景、大上下间距、
/// 黑色加粗居中文案，铺满可用宽度。
pub(crate) fn toast_card(message: SharedString) -> Div {
    let bg: gpui_kit::Hsla = gpui_kit::rgb(0xe4e4e7).into();
    let fg: gpui_kit::Hsla = gpui_kit::rgb(0x000000).into();
    div()
        .w_full()
        .px_6()
        .py(px(22.))
        .rounded_md()
        .bg(bg)
        .text_color(fg)
        .text_center()
        .text_size(px(16.))
        .font_weight(FontWeight::BOLD)
        .shadow_md()
        .child(message)
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
