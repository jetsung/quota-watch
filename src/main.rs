// Release 构建为 GUI 子系统，双击启动不弹 Console；debug（cargo run）保留控制台便于排障。
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod kwin;
mod models;
mod notifications;
mod persistence;
mod timer_engine;
mod tray;
mod update;
mod webdav;
#[cfg(target_os = "windows")]
mod win32;
mod window_control;

mod ui;

use gpui_kit::AppContext as _;
use ui::toast::BottomToast as _;

fn main() {
    // --version / -V：打印版本号后退出，不启动 GUI（供命令行 / 包管理器核对）。
    let mut cli_args = std::env::args();
    if cli_args.next().is_some() && cli_args.any(|a| a == "--version" || a == "-V") {
        // Release（GUI 子系统）无控制台：附加父进程控制台后打印；失败（如双击）静默。
        #[cfg(all(target_os = "windows", not(debug_assertions)))]
        win32::attach_parent_console();
        println!("quota-watch {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // 启动时加载配置（3.4）：解析失败时 persistence 已自动备份并以空状态启动。
    let (persisted, load_warning) = persistence::load();

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx: &mut gpui_kit::App| {
            gpui_kit::init(cx);

            // AppState 初始化（7.1）
            let state = models::new_app_state(cx, persisted);

            // 主窗口可见性标志：由 observe_window_visibility 维护，托盘"显示 / 隐藏"用
            let window_visible = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));

            // 窗口与 RootView 挂载：固定宽度 445px，默认高度 680px（最小 680px，无上限）。
            // 445×680 为请求尺寸，实际观测窗口约 400×640（X11 下存在约 45×40 的差值）。
            // 自绘标题栏（Client 装饰）：移除系统标题栏，三键由软件绘制。
            let (handle, _root) = gpui_kit::open_window(
                gpui_kit::WindowOptions {
                    // Windows：主显示器工作区居中（gpui 的 centered 取 work area，避开任务栏、DPI 归一）；
                    // 其他平台保持原有 (0,0) 行为不变。
                    #[cfg(target_os = "windows")]
                    window_bounds: Some(gpui_kit::WindowBounds::centered(
                        gpui_kit::size(gpui_kit::px(445.), gpui_kit::px(680.)),
                        cx,
                    )),
                    #[cfg(not(target_os = "windows"))]
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                        origin: Default::default(),
                        size: gpui_kit::size(gpui_kit::px(445.), gpui_kit::px(680.)),
                    })),
                    // 最小尺寸 445×680：宽锁定 445，高度可向上调整
                    window_min_size: Some(gpui_kit::size(gpui_kit::px(445.), gpui_kit::px(680.))),
                    // titlebar.title 用于设置 X11 的 WM_NAME/_NET_WM_NAME 与 Wayland 的窗口标题；
                    // appears_transparent=false 且 Client 装饰下不会绘制系统标题栏，自绘标题栏不受影响
                    titlebar: Some(gpui_kit::TitlebarOptions {
                        title: Some("Quota Watch".into()),
                        ..Default::default()
                    }),
                    window_decorations: Some(gpui_kit::WindowDecorations::Client),
                    app_owns_titlebar_drag: true,
                    // Wayland/KWin 下托盘脚本按 resourceClass 匹配窗口
                    app_id: Some("quota-watch".into()),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    // Windows：任务栏按钮图标 = 应用图标（gpui 的 WindowOptions.icon 仅 X11 生效）
                    #[cfg(target_os = "windows")]
                    crate::win32::set_window_icon(window);
                    // 跟踪窗口可见性（最小化/恢复、关闭拦截等所有来源统一更新）
                    let flag = window_visible.clone();
                    window
                        .observe_window_visibility(move |vis, _, _| {
                            flag.store(
                                matches!(vis, gpui_kit::WindowVisibility::Visible),
                                std::sync::atomic::Ordering::Relaxed,
                            );
                        })
                        .detach();
                    cx.new(|cx| ui::RootView::new(state.clone(), window, cx))
                },
            )
            .expect("failed to open window");

            // 启动告警改为 Toast 提示（无日志区域）
            if let Some(w) = load_warning {
                handle
                    .update(cx, |_, win, app| {
                        win.push_toast(
                            gpui_kit::component::notification::Notification::error(w),
                            app,
                        );
                    })
                    .ok();
            }

            // 后台 1s tick 引擎（5.2）：通知失败 → Toast，不中断 tick
            let notifier = std::sync::Arc::new(notifications::DesktopNotificationService);
            let timer_handle = handle;
            let task = timer_engine::spawn_timer(cx, state.clone(), notifier, move |msg, app| {
                let _ = timer_handle.update(app, |_, win, a| {
                    win.push_toast(
                        gpui_kit::component::notification::Notification::error(msg),
                        a,
                    );
                });
            });
            task.detach();
            // 系统托盘常驻：左键切换窗口激活/最小化，右键菜单（显示/隐藏、退出）
            tray::spawn_tray(cx, handle, window_visible).detach();
        });
}
