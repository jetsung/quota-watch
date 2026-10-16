use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::prelude::*;
use gpui_kit::{App, ClickEvent, IntoElement, Window};

/// Footer：左侧全局通知开关，右侧"新建服务"按钮（同一行）。
/// `on_toggle_global`：(&bool, &mut Window, &mut App)；
/// `on_add`：点击"新建服务"的回调。
pub fn render_footer(
    global_enabled: bool,
    on_toggle_global: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_add: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .border_t_1()
        .px_4()
        .py_2()
        .child(
            Checkbox::new("global-notifications")
                .label("启用桌面通知")
                .checked(global_enabled)
                .font_weight(gpui_kit::FontWeight::BOLD)
                .on_change(on_toggle_global),
        )
        .child(
            Button::new("add-service")
                .primary()
                .label("新建服务")
                .font_weight(gpui_kit::FontWeight::BOLD)
                .on_click(on_add),
        )
}
