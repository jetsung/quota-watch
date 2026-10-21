mod kwin;
mod models;
mod notifications;
mod persistence;
mod timer_engine;
mod tray;
mod window_control;

mod ui;

use gpui_kit::AppContext as _;
use gpui_kit::component::WindowExt as _;

fn main() {
    // --version / -V：打印版本号后退出，不启动 GUI（供命令行 / 包管理器核对）。
    let mut cli_args = std::env::args();
    if cli_args.next().is_some() && cli_args.any(|a| a == "--version" || a == "-V") {
        println!("quota-watch {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // 启动时加载配置（3.4）：解析失败时 persistence 已自动备份并以空状态启动。
    let (persisted, load_warning) = persistence::load();

    gpui_kit::application().run(|cx: &mut gpui_kit::App| {
        gpui_kit::init(cx);

        // AppState 初始化（7.1）
        let state = models::new_app_state(cx, persisted);

        // 主窗口可见性标志：由 observe_window_visibility 维护，托盘"显示 / 隐藏"用
        let window_visible = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));

        // 窗口与 RootView 挂载：固定宽度 400px，默认高度 720px（最小 640px，无上限）。
        // 默认高度取 720 以完整容纳"新建/编辑"对话框表单，避免其内部出现滚动条。
        // 自绘标题栏（Client 装饰）：移除系统标题栏，三键由软件绘制。
        let (handle, _root) = gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: Default::default(),
                    size: gpui_kit::size(gpui_kit::px(400.), gpui_kit::px(720.)),
                })),
                // 最小尺寸 400×640：与 817ff0f（系统标题栏时期）一致——
                // 宽锁定 400，高度可向上调整
                window_min_size: Some(gpui_kit::size(gpui_kit::px(400.), gpui_kit::px(640.))),
                titlebar: None,
                window_decorations: Some(gpui_kit::WindowDecorations::Client),
                app_owns_titlebar_drag: true,
                // Wayland/KWin 下托盘脚本按 resourceClass 匹配窗口
                app_id: Some("quota-watch".into()),
                ..Default::default()
            },
            cx,
            |window, cx| {
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
                    win.push_notification(
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
                win.push_notification(
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
