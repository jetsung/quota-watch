mod models;
mod notifications;
mod persistence;
mod timer_engine;
mod ui;

use gpui_kit::AppContext as _;
use gpui_kit::component::WindowExt as _;

fn main() {
    // 启动时加载配置（3.4）：解析失败时 persistence 已自动备份并以空状态启动。
    let (persisted, load_warning) = persistence::load();

    gpui_kit::application().run(|cx: &mut gpui_kit::App| {
        gpui_kit::init(cx);

        // AppState 初始化（7.1）
        let state = models::new_app_state(cx, persisted);

        // 窗口与 RootView 挂载：固定宽度 400px，高度初始 640px（最小高度 640px，无上限）
        // 使用系统级窗口装饰：软件名 "Quota Reset Notifier" 由系统标题栏承载。
        let (handle, _root) = gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: Default::default(),
                    size: gpui_kit::size(gpui_kit::px(400.), gpui_kit::px(640.)),
                })),
                window_min_size: Some(gpui_kit::size(gpui_kit::px(400.), gpui_kit::px(640.))),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("配额重置提醒".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|_| ui::RootView::new(state.clone())),
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
        let timer_handle = handle.clone();
        let task = timer_engine::spawn_timer(cx, state.clone(), notifier, move |msg, app| {
            let _ = timer_handle.update(app, |_, win, a| {
                win.push_notification(
                    gpui_kit::component::notification::Notification::error(msg),
                    a,
                );
            });
        });
        task.detach();
    });
}
