use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, FontWeight, MouseButton, Window};

/// 自绘标题栏：左区应用图标 + 软件名，右区 缩小/放大/关闭 三键。
/// 空白区按下左键拖动移动窗口（双击忽略，不最大化）；
/// 缩小/关闭 = 最小化到托盘（关闭同时从 KWin 任务栏隐藏）；
/// 放大 = 最大化/还原切换（图标随状态联动）。
pub fn render_titlebar(window: &Window, cx: &App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let maximized = window.is_maximized();
    h_flex()
        .id("titlebar")
        .w_full()
        .h(gpui_kit::px(36.))
        .items_center()
        .justify_between()
        .px_2()
        .bg(theme.background)
        .border_b_1()
        .border_color(theme.border)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            // 双击空白区不触发最大化（窗口固定尺寸），仅拖动。
            if event.click_count <= 1 {
                window.start_window_move();
            }
        })
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    app_icon()
                        .text_color(theme.foreground)
                        .with_size(gpui_kit::component::Size::Small),
                )
                .child(
                    gpui_kit::div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child("配额重置提醒"),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap(gpui_kit::px(2.))
                // 按钮区拦截左键按下，阻止冒泡到标题栏拖动
                // （否则 start_window_move 会吞掉按钮的点击）
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                // 缩小：可用态，托盘常驻语义不变
                .child(
                    Button::new("titlebar-minimize")
                        .ghost()
                        .icon(Icon::default().data(MINUS_SVG))
                        .tooltip("最小化")
                        .accessibility_label("最小化")
                        .on_click(|_, window, _| window.minimize_window()),
                )
                // 放大：最大化/还原切换，图标与文案随状态联动
                .child(
                    Button::new("titlebar-maximize")
                        .ghost()
                        .icon(Icon::default().data(if maximized {
                            RESTORE_SVG
                        } else {
                            SQUARE_SVG
                        }))
                        .tooltip(if maximized { "还原" } else { "最大化" })
                        .accessibility_label(if maximized { "还原" } else { "最大化" })
                        .on_click(|_, window, _| window.zoom_window()),
                )
                // 关闭：等同"最小化到托盘"，与 on_window_should_close 拦截一致；
                // 同时从 KWin 任务栏隐藏（托盘激活后恢复）
                .child(
                    Button::new("titlebar-close")
                        .ghost()
                        .icon(Icon::default().data(CLOSE_SVG))
                        .tooltip("关闭")
                        .accessibility_label("关闭")
                        .custom(ButtonCustomVariant::new(cx).hover(theme.danger.opacity(0.15)))
                        .on_click(|_, window, _| {
                            window.minimize_window();
                            std::thread::spawn(crate::kwin::hide_via_kwin);
                        }),
                ),
        )
}

/// 左区应用图标：内联 `assets/icons/quota-watch.svg`，无需注册 asset 路径。
fn app_icon() -> Icon {
    Icon::default().data(include_str!("../../assets/icons/quota-watch.svg").as_bytes())
}

/// 三个窗口控制键的内联 SVG（Lucide 风格，currentColor 描边）。
const MINUS_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/></svg>"##;
const SQUARE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="5" y="5" rx="2"/></svg>"##;
const CLOSE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"##;
/// 还原（两个重叠方框）图标内联 SVG（Lucide copy）。
const RESTORE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>"##;
