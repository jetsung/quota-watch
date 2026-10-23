use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, ClickEvent, FontWeight, IntoElement, Window};

/// Footer：仅保留"新建服务"按钮（通知开关已迁移至设置面板）。
pub fn render_footer(
    on_add: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    _cx: &App,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_end()
        .items_center()
        .border_t_1()
        .px_4()
        .py_2()
        .child(
            Button::new("add-service")
                .primary()
                .label("新建服务")
                .font_weight(FontWeight::BOLD)
                .on_click(on_add),
        )
}
