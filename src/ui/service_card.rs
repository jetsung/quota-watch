use crate::models::{ServiceEntry, format_countdown};
use crate::ui::checkbox::render_checkbox;
use chrono::Utc;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, IntoElement, MouseDownEvent, ParentElement, Styled, Window, div,
    px,
};

use std::rc::Rc;

/// 卡片操作回调类型（复杂函数指针类型的别名，避免字段处触发 type_complexity）。
type MuteCallback = Rc<dyn Fn(&bool, &mut Window, &mut App) + Send + Sync>;
type ClickCallback = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + Send + Sync>;
type MouseDownCallback = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + Send + Sync>;

/// 单张服务卡片的操作回调（由 RootView 用 cx.listener 注入）。
/// 组件 API 要求 Sized 的 `impl Fn`，故字段持有 `Rc<dyn Fn>`，渲染时包一层具体闭包。
pub struct CardActions {
    pub on_toggle_mute: MuteCallback,
    pub on_toggle_pin: MuteCallback,
    pub on_edit: ClickCallback,
    pub on_delete: ClickCallback,
    pub on_delete_context: MouseDownCallback,
}

/// 渲染单张服务卡片：服务名、状态标签、HH:MM:SS 倒计时、
/// 阈值内切换 warning/danger 主题色、Mute Checkbox、Edit/Delete 按钮、右键删除。
/// 纯函数生成元素（RenderOnce 语义：静态部分每次重建成本可忽略，仅倒计时文本随 tick 变化）。
/// `stretch`：多列布局时卡片在 h_flex 行内等宽伸缩（单列列表保持 w_full 即可）。
pub fn render_service_card(
    entry: &ServiceEntry,
    actions: CardActions,
    cx: &App,
    stretch: bool,
) -> AnyElement {
    let now = Utc::now();
    let remaining = entry.time_until_reset(now);
    let remaining_secs = remaining.num_seconds();
    let threshold_secs = entry.reminder_threshold_minutes as i64 * 60;
    let urgent = remaining_secs <= threshold_secs;
    let theme = cx.theme();
    let border_color = if urgent {
        if remaining_secs <= 0 {
            theme.danger
        } else {
            theme.warning
        }
    } else {
        theme.border
    };
    // 状态徽章：emoji + 白色粗体文字 + 纯色背景。
    // 正常=用户指定绿（#088f4e），即将重置=褐色，重置中=danger 红。
    let green_bg: gpui_kit::Hsla = gpui_kit::rgb(0x088f4e).into();
    // 褐色（即将重置）：主题体系无 brown token，使用固定深褐色
    let brown = gpui_kit::hsla(0.07, 0.72, 0.38, 1.0);
    let (status_text, badge_bg) = if remaining_secs <= 0 {
        ("● 重置中", theme.colors.danger)
    } else if urgent {
        ("☆ 即将重置", brown)
    } else {
        ("★ 正常", green_bg)
    };
    let countdown = if remaining_secs <= 0 {
        "00:00:00".to_string()
    } else {
        format_countdown(remaining)
    };
    let next_local = entry
        .next_reset_at()
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let id = entry.id;

    let mut card = v_flex()
        .id(id)
        .w_full()
        .bg(theme.colors.secondary)
        .border_1()
        .border_color(border_color)
        .when(urgent, |el| el.bg(cx.theme().warning.opacity(0.08)))
        .rounded_md()
        .p_3()
        .gap_2();
    // 多列模式：在 h_flex 行内等宽伸缩
    if stretch {
        card = card.flex_grow_1().flex_shrink_1().min_w_0();
    }
    card.on_mouse_down(gpui_kit::MouseButton::Right, {
        let f = actions.on_delete_context;
        move |e: &MouseDownEvent, w: &mut Window, a: &mut App| f(e, w, a)
    })
    .child(
        h_flex()
            .justify_between()
            .items_center()
            .child({
                // id 前置使两分支同为 Stateful<Div>；配置了 URL 的条目标题可点击，在系统浏览器打开
                // Linux 主 UI 字体不含 U+1F4CC（📌）字形且 GPUI 无 emoji 回退，改用 BMP 的 ▲
                let title_text = if entry.is_pinned {
                    let marker = if cfg!(target_os = "linux") {
                        "▲"
                    } else {
                        "📌"
                    };
                    format!("{} {}", marker, entry.service_name)
                } else {
                    entry.service_name.clone()
                };
                let title = div()
                    .id(format!("title-{}", id))
                    .text_size(px(18.))
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .child(title_text);
                match entry.url.clone() {
                    Some(url) => title
                        .cursor_pointer()
                        .text_color(theme.colors.primary)
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                    None => title,
                }
            })
            .child(
                h_flex()
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .bg(badge_bg)
                    .text_size(px(14.))
                    .text_color(gpui_kit::white())
                    .child(status_text),
            ),
    )
    .child(
        div()
            .text_size(px(28.))
            .font_weight(gpui_kit::FontWeight::BOLD)
            .child(countdown),
    )
    .child(
        h_flex()
            .text_size(px(14.))
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .child("下次重置："),
            )
            .child(div().child(next_local)),
    )
    .child(
        h_flex()
            // 组间 16px：与组内 4px（框-文字）、按钮间 8px 拉开层次，
            // 避免"勾选框前后文本等距"导致分组不直观
            .gap_4()
            .items_center()
            .child(render_checkbox(
                format!("mute-{}", id),
                "静音重复提醒",
                entry.is_muted,
                {
                    let f = actions.on_toggle_mute;
                    move |c: &bool, w: &mut Window, a: &mut App| f(c, w, a)
                },
                Some(px(13.)),
                cx,
            ))
            .child(render_checkbox(
                format!("pin-{}", id),
                "置顶",
                entry.is_pinned,
                {
                    let f = actions.on_toggle_pin;
                    move |c: &bool, w: &mut Window, a: &mut App| f(c, w, a)
                },
                Some(px(13.)),
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child({
                        let border = theme.colors.border;
                        div()
                            .id(format!("edit-{}", id))
                            .text_size(px(13.))
                            .text_color(theme.colors.foreground)
                            .cursor_pointer()
                            .px_2()
                            .rounded_sm()
                            .border_1()
                            .border_color(gpui_kit::hsla(0., 0., 0., 0.))
                            .hover(move |s| s.bg(gpui_kit::white()).border_1().border_color(border))
                            .child("编辑")
                            .on_click({
                                let f = actions.on_edit;
                                move |e: &ClickEvent, w: &mut Window, a: &mut App| f(e, w, a)
                            })
                    })
                    .child({
                        let border = theme.colors.border;
                        div()
                            .id(format!("delete-{}", id))
                            .text_size(px(13.))
                            .text_color(theme.colors.danger)
                            .cursor_pointer()
                            .px_2()
                            .rounded_sm()
                            .border_1()
                            .border_color(gpui_kit::hsla(0., 0., 0., 0.))
                            .hover(move |s| s.bg(gpui_kit::white()).border_1().border_color(border))
                            .child("删除")
                            .on_click({
                                let f = actions.on_delete;
                                move |e: &ClickEvent, w: &mut Window, a: &mut App| f(e, w, a)
                            })
                    }),
            ),
    )
    .into_any_element()
}
