use gpui_kit::App;
use gpui_kit::base::h_flex;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;

/// 作者主页：点击状态栏作者名称时跳转。
const AUTHOR_URL: &str = "https://git.jetsung.com/jetsung/quota-watch";

/// 状态栏文字色：比 theme.secondary 更深的灰（主前景色 60% 透明度）。
fn statusbar_text_color(theme: &gpui_kit::component::theme::Theme) -> gpui_kit::Hsla {
    theme.foreground.opacity(0.6)
}

/// 状态栏：左侧软件版本号，右侧版权信息（作者名称可点击跳转）。
pub fn render_statusbar(cx: &App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let text_color = statusbar_text_color(&theme);
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .px_4()
        .py_1()
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.background)
        .text_xs()
        .child(
            gpui_kit::div()
                .text_color(text_color)
                .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
        )
        .child(
            h_flex()
                .items_center()
                .gap(gpui_kit::px(2.))
                .child(gpui_kit::div().text_color(text_color).child("© 2026"))
                .child(
                    // 作者名称：可点击跳转，无下划线，颜色与 © 2026 一致
                    gpui_kit::div()
                        .id("statusbar-author")
                        .text_color(text_color)
                        .cursor_pointer()
                        .on_click(|_, _window, cx| {
                            cx.open_url(AUTHOR_URL);
                        })
                        .child("Jetsung Chan"),
                ),
        )
}
