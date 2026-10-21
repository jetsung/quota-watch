//! 系统托盘常驻：托盘图标 + 右键菜单（打开/隐藏、退出）。
//!
//! 使用 `tray-icon` crate（Linux 走 KSNI 后端，D-Bus StatusNotifierItem，
//! 不依赖 GTK 事件循环，与 GPUI 的 calloop 事件循环兼容）。
//!
//! 事件处理：`TrayIconEvent`（托盘点击）和 `MenuEvent`（菜单项）通过
//! crossbeam channel 投递；在 `cx.spawn` 的后台 task 中轮询消费，
//! 通过 `AnyWindowHandle` 操作窗口激活/最小化。
//! 左键单击激活窗口（置顶显示）。

use gpui_kit::{AnyWindowHandle, App, Task};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};

/// 托盘图标 ID。
const TRAY_ID: &str = "quota-watch-tray";
/// 菜单项 ID：显示/隐藏（按窗口当前状态切换）。
const MENU_TOGGLE: &str = "toggle";
/// 菜单项 ID：退出应用。
const MENU_QUIT: &str = "quit";

/// 加载嵌入的 PNG 图标为 `tray_icon::Icon`。
fn load_icon() -> Icon {
    let png = include_bytes!("../assets/icons/quota-watch-128.png");
    let img = image::load_from_memory(png)
        .expect("加载托盘图标失败")
        .to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).expect("托盘图标 RGBA 数据无效")
}
fn restore_and_activate(win: &mut gpui_kit::Window) {
    // X11/EWMH 与 KWin 逻辑统一在 window_control 中（KWin 优先、X11 回退）
    crate::window_control::restore_from_tray(win);
}

/// 创建系统托盘并启动事件轮询 task。
///
/// 返回的 `Task` 持有托盘事件循环；detach 后与应用同生命周期。
/// `window`：主窗口句柄，用于激活/最小化切换。
/// `visible`：主窗口可见性标志（由 `observe_window_visibility` 维护），
/// 用于"显示 / 隐藏"菜单项的状态反转切换。
pub fn spawn_tray(cx: &mut App, window: AnyWindowHandle, visible: Arc<AtomicBool>) -> Task<()> {
    // 构建右键菜单：文字固定为"打开 / 隐藏"，行为按窗口当前状态取反
    let menu = Menu::with_items(&[
        &MenuItem::with_id(MENU_TOGGLE, "打开 / 隐藏", true, None),
        &MenuItem::with_id(MENU_QUIT, "退出", true, None),
    ])
    .expect("创建托盘菜单失败");
    let tray = TrayIconBuilder::new()
        .with_id(TRAY_ID)
        .with_tooltip("配额重置提醒")
        .with_icon(load_icon())
        .with_menu(Box::new(menu))
        .build()
        .expect("创建托盘图标失败");

    // tray 移入 async 块保持存活；Drop 会移除托盘图标，必须与应用同生命周期。
    cx.spawn(async move |cx| {
        let _tray = tray;
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;

            // 托盘左键单击：激活窗口（Wayland 下走 KWin 脚本通道才可靠）
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click { button, .. } = &event
                    && *button == tray_icon::MouseButton::Left
                {
                    let _ = window.update(cx, |_, win, app| {
                        restore_and_activate(win);
                        app.activate(true);
                    });
                    visible.store(true, Ordering::Relaxed);
                }
            }

            // 菜单事件：显示/隐藏（按窗口状态取反切换）/ 退出
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                match event.id.0.as_str() {
                    MENU_TOGGLE => {
                        // 可见 → 隐藏（最小化到托盘，语义与关闭一致）；
                        // 隐藏/最小化 → 恢复、置顶并激活
                        if visible.load(Ordering::Relaxed) {
                            let _ = window
                                .update(cx, |_, win, _| crate::window_control::hide_to_tray(win));
                            visible.store(false, Ordering::Relaxed);
                        } else {
                            let _ = window.update(cx, |_, win, app| {
                                restore_and_activate(win);
                                app.activate(true);
                            });
                            visible.store(true, Ordering::Relaxed);
                        }
                    }
                    MENU_QUIT => {
                        cx.update(|app| app.quit());
                    }
                    _ => {}
                }
            }
        }
    })
}
