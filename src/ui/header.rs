use gpui_kit::base::h_flex;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, ClickEvent, IntoElement, Window};

/// Header：顶部工具栏，三平台一致渲染（系统标题栏不再承载应用内入口）。
/// - 左侧：设置按钮（图标按钮，打开设置面板）；
/// - 右侧：新建服务按钮。
pub fn render_header(
    on_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_add: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .px_4()
        .py_2()
        .border_b_1()
        .child(
            Button::new("header-settings")
                .ghost()
                .icon(Icon::default().data(MENU_SVG))
                .tooltip("设置")
                .accessibility_label("设置")
                .on_click(on_settings),
        )
        .child(
            Button::new("add-service")
                .primary()
                .label("新建服务")
                .on_click(on_add),
        )
}

/// 设置菜单图标内联 SVG（Lucide menu 风格，currentColor 描边）。
const MENU_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 6h16"/><path d="M4 12h16"/><path d="M4 18h16"/></svg>"##;
