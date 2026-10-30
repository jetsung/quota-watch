mod checkbox;
mod modal;
mod service_card;
mod service_list;
mod settings;
pub mod statusbar;
pub(crate) mod toast;

pub use modal::AddEditView;
use settings::SettingsTab;
pub use settings::SettingsView;
use toast::ToastState;

use crate::models::{self, AppState, ServiceEntry};
use gpui_kit::base::v_flex;
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, Entity, IntoElement, MouseDownEvent, ParentElement, Render,
    ScrollHandle, SharedString, Window, point, px,
};
use std::rc::Rc;
use uuid::Uuid;

/// RootView（6.1）：读取 `Entity<AppState>`，组合服务列表与状态栏，
/// 处理空状态占位与引导；承载 Add/Edit Modal、删除确认与全局开关交互。
/// 设置与新增服务入口由状态栏左下角"设置"菜单提供。
pub struct RootView {
    state: Entity<AppState>,
    toast: Option<ToastState>,
    toast_seq: u64,
    /// 服务列表滚动容器 handle：置顶后据此把列表滚回顶部。
    list_scroll_handle: ScrollHandle,
}

impl RootView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &App) -> Self {
        // 拦截窗口关闭：返回 false 阻止关闭，改为"最小化到托盘"（KWin 优先、
        // X11/EWMH 回退隐藏任务栏图标），与标题栏"关闭"一致。
        window.on_window_should_close(cx, |window, _cx| {
            crate::window_control::hide_to_tray(window);
            false
        });
        // gpui-fast X11 后端会在 GPU 初始化完成（异步）后写入自己的
        // WM_SIZE_HINTS（max_size 恒为纹理上限），同步写会被其覆盖。
        // 因此延迟并多次重试写入，确保最终落在后端写入之后：
        // 锁宽 445（min==max），高度 [680, 上限] 可调；此后 WM 直接拒绝
        // 宽度方向的拖拽（无橡皮筋效果，不依赖每帧钳制）。
        let window_handle = window.window_handle();
        cx.spawn(async move |app| {
            for _ in 0..10 {
                app.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let _ = window_handle.update(app, |_, win, _| Self::lock_window_width_x11(win));
            }
        })
        .detach();
        Self {
            state,
            toast: None,
            toast_seq: 0,
            list_scroll_handle: ScrollHandle::new(),
        }
    }

    /// 在窗口正中显示提示（淡入上浮）；2 秒后淡出下沉并移除。
    /// 序号防止旧定时器干扰新提示。
    pub(crate) fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast_seq = self.toast_seq.wrapping_add(1);
        let seq = self.toast_seq;
        self.toast = Some(ToastState {
            seq,
            message: message.into(),
            exiting: false,
            action_upgrade: false,
            action_restart: false,
            action_cancel: false,
            persistent: false,
        });
        cx.notify();
        cx.spawn(async move |this, async_cx| {
            async_cx
                .background_executor()
                .timer(std::time::Duration::from_millis(2000))
                .await;
            let _ = async_cx.update(|cx| {
                this.update(cx, |view, cx| {
                    if view.toast.as_ref().map(|toast| toast.seq) == Some(seq) {
                        let persistent = view.toast.as_ref().unwrap().persistent;
                        if !persistent {
                            view.toast.as_mut().unwrap().exiting = true;
                            cx.notify();
                        }
                    }
                })
            });
            async_cx
                .background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let _ = async_cx.update(|cx| {
                this.update(cx, |view, cx| {
                    if view.toast.as_ref().map(|toast| toast.seq) == Some(seq) {
                        let persistent = view.toast.as_ref().unwrap().persistent;
                        if !persistent {
                            view.toast = None;
                            cx.notify();
                        }
                    }
                })
            });
        })
        .detach();
    }

    fn lock_window_width_x11(window: &Window) {
        #[cfg(all(target_os = "linux", not(target_os = "android")))]
        {
            use x11rb::connection::Connection as _;
            use x11rb::wrapper::ConnectionExt as _2;
            if let Ok(handle) =
                <Window as raw_window_handle::HasWindowHandle>::window_handle(window)
            {
                let window_id: Option<u32> = match handle.as_raw() {
                    raw_window_handle::RawWindowHandle::Xlib(h) => Some(h.window as u32),
                    raw_window_handle::RawWindowHandle::Xcb(h) => Some(h.window.get()),
                    _ => None,
                };
                if let Some(wid) = window_id
                    && let Ok((conn, _)) = x11rb::connect(None)
                {
                    // WM_SIZE_HINTS: 锁宽 445（min_width == max_width），
                    // 高度 [680, 16384] 可调；flags=0x30 (PMinSize|PMaxSize)
                    let _ = conn.change_property32(
                        x11rb::protocol::xproto::PropMode::REPLACE,
                        wid,
                        x11rb::protocol::xproto::AtomEnum::WM_NORMAL_HINTS,
                        x11rb::protocol::xproto::AtomEnum::WM_SIZE_HINTS,
                        &[
                            0x30u32, // flags: PMinSize|PMaxSize
                            0, 0, 0, 0, 445, 680, 445, 16384, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        ],
                    );
                    let _ = conn.flush();
                }
            }
        }
        let _ = window;
    }

    /// 打开 Add/Edit 对话框（6.2 / 6.5）。
    fn open_add_edit(
        &mut self,
        editing: Option<ServiceEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.state.clone();
        let host = cx.entity();
        let title = match &editing {
            Some(e) => format!("编辑服务：{}", e.service_name),
            None => "添加服务".to_string(),
        };
        // open_dialog 的 builder 是渲染闭包，重建时会再次执行：
        // 实体必须在打开对话框之前创建，builder 仅引用已有实体句柄。
        let view = cx.new(|cx| AddEditView::new(window, cx, state, host, editing, title));
        // 一次性初始焦点 guard：仅在 builder 首次执行时聚焦服务名输入框
        let initial_focus = std::cell::Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                view.read(cx).name_focus_handle(cx).focus(window, cx);
            }
            dialog
                .overlay(true)
                .overlay_closable(false)
                .close_button(false)
                .child(view.clone())
        });
    }

    /// 打开设置单页窗体（page 指定页面，经左下角设置菜单进入）。
    fn open_settings(&mut self, page: SettingsTab, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.clone();
        let view = cx.new(|cx| SettingsView::new(window, cx, state, page));
        window.open_dialog(cx, move |dialog, window, _| {
            // overlay_closable(false)：点击遮罩/主窗体不关闭，仅可通过面板上的“关闭”按钮退出
            // 垂直居中：按视口高度与窗体总高（内容区 + 标题栏/内边距固定增量）换算 margin_top，
            // 对齐删除确认弹窗的换算先例；窗口过矮时保底 24px。
            let total_h =
                px(page.content_height()) + px(crate::ui::settings::SETTINGS_DIALOG_CHROME);
            let margin_top = ((window.viewport_size().height - total_h) / 2.).max(px(24.));
            dialog
                .overlay(true)
                .overlay_closable(false)
                .close_button(false)
                .width(px(520.))
                .margin_top(margin_top)
                .child(view.clone())
        });
    }

    /// 删除确认（6.7）：确认后移除 + 持久化 + Toast；取消不做任何变更。
    fn confirm_delete(&self, id: Uuid, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.clone();
        let host = cx.entity();
        window.open_alert_dialog(cx, move |alert, win, _| {
            let state2 = state.clone();
            let host2 = host.clone();
            // 默认偏移为视口高度的 1/10（靠顶）；按估算的确认框高度换算为垂直居中
            let margin_top = ((win.viewport_size().height - px(150.)) / 2.).max(px(24.));
            alert
                .margin_top(margin_top)
                .title("删除服务")
                .description(format!("确定删除 “{}”？此操作不可撤销。", name))
                .ok_text("确定")
                .cancel_text("取消")
                .confirm()
                .on_ok(move |_, _win, app| {
                    let state3 = state2.clone();
                    let host3 = host2.clone();
                    let host4 = host2.clone();
                    let result = models::mutate(
                        &state3,
                        app,
                        move |s, _| Ok(s.remove(id)),
                        move |msg, app| {
                            host3.update(app, |view, cx| view.show_toast(msg, cx));
                        },
                    );
                    if result == Some(true) {
                        host4.update(app, |view, cx| view.show_toast("已删除", cx));
                    }
                    true
                })
        });
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = (*self.state.read(cx)).clone();
        // 设置/新增编辑等对话框打开时，用遮罩盖住主窗体，避免误点下层元素
        let has_active_dialog = gpui_kit::component::WindowExt::has_active_dialog(window, cx);
        let state_ent = self.state.clone();

        // ServiceList（6.3 / 6.4）：每个 id 构造 CardActions
        let root_entity = cx.entity();
        let make_actions = |id: Uuid| -> service_card::CardActions {
            let state2 = state_ent.clone();
            let entry = state.services.iter().find(|s| s.id == id).cloned();
            service_card::CardActions {
                on_toggle_mute: {
                    let state3 = state2.clone();
                    let root = root_entity.clone();
                    let name = entry
                        .as_ref()
                        .map(|x| x.service_name.clone())
                        .unwrap_or_default();
                    Rc::new(move |muted, _, app| {
                        let muted = *muted;
                        let name = name.clone();
                        let result = models::mutate(&state3, app, move |s, _| s.toggle_mute(id), {
                            let root = root.clone();
                            move |msg, app| {
                                root.update(app, |view, cx| view.show_toast(msg, cx));
                            }
                        });
                        if result.is_some() {
                            root.update(app, move |view, cx| {
                                view.show_toast(
                                    format!(
                                        "已{} {} 的静音",
                                        if muted { "开启" } else { "关闭" },
                                        name
                                    ),
                                    cx,
                                )
                            });
                        }
                    })
                },
                on_toggle_pin: {
                    let state3 = state2.clone();
                    let root = root_entity.clone();
                    let name = entry
                        .as_ref()
                        .map(|x| x.service_name.clone())
                        .unwrap_or_default();
                    // ScrollHandle（Rc 内核）非 Send+Sync，不能被回调闭包捕获；
                    // 经 root.update 在主线程回调内通过 view 访问
                    Rc::new(move |pinned, _, app| {
                        let pinned = *pinned;
                        let name = name.clone();
                        let result = models::mutate(&state3, app, move |s, _| s.toggle_pin(id), {
                            let root = root.clone();
                            move |msg, app| {
                                root.update(app, |view, cx| view.show_toast(msg, cx));
                            }
                        });
                        if result.is_some() {
                            root.update(app, move |view, cx| {
                                // 勾选置顶后条目已排至列表首位，滚动位置跟随回顶；
                                // 取消置顶不动滚动位置
                                if pinned {
                                    view.list_scroll_handle.set_offset(point(px(0.), px(0.)));
                                }
                                view.show_toast(
                                    format!("已{}置顶 {}", if pinned { "" } else { "取消" }, name),
                                    cx,
                                )
                            });
                        }
                    })
                },
                on_edit: {
                    let e = entry.clone();
                    Rc::new(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if let Some(e) = e.as_ref() {
                            this.open_add_edit(Some(e.clone()), window, cx);
                        }
                    }))
                },
                on_delete: {
                    let e = entry.clone();
                    Rc::new(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        let (id2, name) = e
                            .as_ref()
                            .map(|x| (x.id, x.service_name.clone()))
                            .unwrap_or_default();
                        this.confirm_delete(id2, name, window, cx);
                    }))
                },
                on_delete_context: {
                    let e = entry.clone();
                    Rc::new(cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        let (id2, name) = e
                            .as_ref()
                            .map(|x| (x.id, x.service_name.clone()))
                            .unwrap_or_default();
                        this.confirm_delete(id2, name, window, cx);
                    }))
                },
            }
        };
        let list = service_list::render_service_list(
            &state,
            &make_actions,
            window.viewport_size().width.into(),
            &self.list_scroll_handle,
            cx,
        );

        let toast_state = self.toast.clone();

        v_flex()
            .relative()
            .size_full()
            // 三平台统一：系统标题栏负责窗口控制，服务入口由左下角设置菜单提供。
            .child(list)
            .child(statusbar::render_statusbar(
                cx,
                cx.listener(
                    |this, action: &statusbar::SettingsAction, window, cx| match action {
                        statusbar::SettingsAction::AddService => {
                            this.open_add_edit(None, window, cx);
                        }
                        statusbar::SettingsAction::OpenPage(page) => {
                            this.open_settings(*page, window, cx);
                        }
                    },
                ),
            ))
            .when(has_active_dialog, |el| {
                el.child(
                    gpui_kit::div()
                        .id("dialog-scrim")
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .bg(gpui_kit::hsla(0., 0., 0., 0.18))
                        .occlude()
                        .on_click(|_: &gpui_kit::ClickEvent, _, _| {}),
                )
            })
            .when_some(toast_state, |el, state| {
                el.child(toast::toast_overlay(&state, None, true, cx))
            })
    }
}
