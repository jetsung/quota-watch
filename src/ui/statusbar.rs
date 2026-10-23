use gpui_kit::App;
use gpui_kit::base::h_flex;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;

/// 状态栏文字色：比 theme.secondary 更深的灰（主前景色 60% 透明度）。
fn statusbar_text_color(theme: &gpui_kit::component::theme::Theme) -> gpui_kit::Hsla {
    theme.foreground.opacity(0.6)
}

/// 状态栏：左侧软件版本号，右侧版权信息（作者与链接已收敛至设置“关于”）。
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
        .child(gpui_kit::div().text_color(text_color).child("© 2026"))
}
