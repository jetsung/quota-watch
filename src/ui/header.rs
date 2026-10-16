use gpui_kit::base::h_flex;
use gpui_kit::prelude::*;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{App, ClickEvent, Window};

/// Header：右侧"新建服务"按钮（软件名显示在系统标题栏，此处不再重复）。
/// 点击按钮回调由调用方注入（需要 RootView 状态，用 cx.listener 包装后传入）。
pub fn render_header(
    on_add: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_end()
        .items_center()
        .px_4()
        .py_3()
        .border_b_1()
        .child(
            Button::new("add-service")
                .primary()
                .label("新建服务")
                .on_click(on_add),
        )
}
