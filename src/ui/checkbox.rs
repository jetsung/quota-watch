use gpui_kit::base::h_flex;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ElementId, IntoElement, ParentElement, Pixels, Styled, Window, div, px, white,
};

/// 自定义勾选框：勾选时显示“✓”字符（gpui-kit 的 Checkbox 组件勾选图标为硬编码
/// SVG，无法定制为字符，故自建）。
///
/// 行为：点击切换，`on_change` 回调接收新的勾选值。
pub fn render_checkbox(
    id: impl Into<ElementId>,
    label: &str,
    checked: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
    text_size: Option<Pixels>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let label = label.to_string();
    // 文字默认 13px；行高设为与字号相等（行框紧贴字形），配合 items_center
    // 使 14px 方框与文字真正垂直居中（默认 1.5 倍行高会导致框与文字上下不齐）
    let text_size = text_size.unwrap_or(px(13.));
    // 14px 方框（贴近 13px 文字高度，避免框明显高出文字）：未勾选 = 前景色半透明
    // 边框（input/border 色与灰色卡片背景过于接近，视觉上不直观）；
    // 勾选 = 主色背景 + 边框 + 白色“✓”
    let mut box_ = div()
        .size_3p5()
        .rounded_sm()
        .border_1()
        .flex()
        .items_center()
        .justify_center()
        .border_color(theme.foreground.opacity(0.45));
    if checked {
        box_ = box_
            .bg(theme.primary)
            .border_color(theme.primary)
            .child(div().text_size(px(11.)).text_color(white()).child("✓"));
    }
    h_flex()
        .id(id)
        .gap_1()
        .items_center()
        .cursor_pointer()
        .on_click(move |_, window, app| on_change(&!checked, window, app))
        .child(box_)
        .child(
            div()
                .text_color(theme.foreground)
                .text_size(text_size)
                .line_height(text_size)
                .child(label),
        )
}
