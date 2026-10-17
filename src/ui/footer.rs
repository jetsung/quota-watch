use crate::ui::checkbox::render_checkbox;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, ClickEvent, FontWeight, IntoElement, Window};

/// Footer：左侧全局通知开关，右侧"新建服务"按钮（同一行）。
/// `on_toggle_global`：(&bool, &mut Window, &mut App)；
/// `on_add`：点击"新建服务"的回调。
pub fn render_footer(
    global_enabled: bool,
    on_toggle_global: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_add: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .border_t_1()
        .px_4()
        .py_2()
        .child(render_checkbox(
            "global-notifications",
            "启用桌面通知",
            global_enabled,
            on_toggle_global,
            cx,
        ))
        .child(
            Button::new("add-service")
                .primary()
                .label("新建服务")
                .font_weight(FontWeight::BOLD)
                .on_click(on_add),
        )
}
