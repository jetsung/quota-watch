use gpui_kit::base::h_flex;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ElementId, FontWeight, IntoElement, ParentElement, Pixels, Styled, Window, div, px, white,
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
    // 16px 方框：未勾选 = 主题输入边框；勾选 = 主色背景 + 白色“✓”
    let mut box_ = div()
        .size_4()
        .rounded_sm()
        .border_1()
        .flex()
        .items_center()
        .justify_center()
        .border_color(theme.input);
    if checked {
        box_ = box_.bg(theme.primary).border_color(theme.primary).child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::BOLD)
                .text_color(white())
                .child("✓"),
        );
    }
    h_flex()
        .id(id)
        .gap_2()
        .items_center()
        .cursor_pointer()
        .on_click(move |_, window, app| on_change(&!checked, window, app))
        .child(box_)
        .child(
            div()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.foreground)
                .when_some(text_size, |el, size| el.text_size(size))
                .child(label),
        )
}
