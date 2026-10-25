use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::prelude::*;
use gpui_kit::{Anchor, App, ClickEvent, IntoElement, Window, px};

use crate::ui::settings::SettingsTab;

/// 设置菜单动作分发回调：菜单项点击后交由宿主打开对应窗体。
type OnSettingsAction = dyn Fn(&SettingsAction, &mut Window, &mut App);

/// 设置菜单项选择结果：由宿主（RootView）分发打开对应窗体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    /// 新增服务（复用现有 Add/Edit 对话框）。
    AddService,
    /// 打开对应的设置单页窗体。
    OpenPage(SettingsTab),
}

/// 状态栏文字色：比 theme.secondary 更深的灰（主前景色 60% 透明度）。
fn statusbar_text_color(theme: &gpui_kit::component::theme::Theme) -> gpui_kit::Hsla {
    theme.foreground.opacity(0.6)
}

/// 设置菜单项统一宽度（容纳五项中文文字并留出余量）。
const MENU_WIDTH: f32 = 120.;

/// 状态栏：左侧（左下角）"设置"按钮弹出设置菜单，右侧显示版本号
/// （版权信息与"新增服务"顶部入口已移除，见"设置菜单"分发）。
pub fn render_statusbar(
    cx: &App,
    on_action: impl Fn(&SettingsAction, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let text_color = statusbar_text_color(&theme);
    let on_action: Rc<OnSettingsAction> = Rc::new(on_action);

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
            // 设置菜单锚定按钮左下角（Anchor::BottomLeft → 面板在按钮上方，
            // 左对齐），从窗口左下角向上弹出。
            Popover::new("statusbar-settings")
                .anchor(Anchor::BottomLeft)
                .trigger(
                    Button::new("statusbar-settings-trigger")
                        .ghost()
                        .icon(Icon::default().data(MENU_SVG))
                        .tooltip("设置")
                        .accessibility_label("设置"),
                )
                .content(move |_, _window, cx| {
                    // 菜单项点击后需主动关闭弹出层：Popover 不随 content 内
                    // 按钮点击自动收起，不关闭会残留菜单并可再开 N 个窗体。
                    let popover = cx.entity();
                    let item = |id: &'static str, label: &'static str, action: SettingsAction| {
                        let on_action = on_action.clone();
                        let popover = popover.clone();
                        Button::new(id)
                            .ghost()
                            .w_full()
                            .px_1()
                            .label(label)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                on_action(&action, window, cx);
                                popover.update(cx, |state, cx| {
                                    state.dismiss(window, cx);
                                });
                            })
                    };
                    v_flex()
                        .w(px(MENU_WIDTH))
                        .gap_1()
                        .child(item(
                            "settings-menu-add-service",
                            "新增服务",
                            SettingsAction::AddService,
                        ))
                        .child(item(
                            "settings-menu-notifications",
                            "桌面通知",
                            SettingsAction::OpenPage(SettingsTab::Notifications),
                        ))
                        .child(item(
                            "settings-menu-backup-sync",
                            "备份与恢复",
                            SettingsAction::OpenPage(SettingsTab::BackupSync),
                        ))
                        .child(item(
                            "settings-menu-import-export",
                            "导入与导出",
                            SettingsAction::OpenPage(SettingsTab::ImportExport),
                        ))
                        .child(item(
                            "settings-menu-about",
                            "关于",
                            SettingsAction::OpenPage(SettingsTab::About),
                        ))
                }),
        )
        .child(
            gpui_kit::div()
                .text_color(text_color)
                .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
        )
}

/// 设置菜单图标内联 SVG（Lucide menu 风格，currentColor 描边）。
const MENU_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 6h16"/><path d="M4 12h16"/><path d="M4 18h16"/></svg>"##;
