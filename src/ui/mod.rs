mod checkbox;
mod footer;
mod modal;
mod service_card;
mod service_list;
pub mod statusbar;
pub mod titlebar;

pub use modal::AddEditView;

use crate::models::{self, AppState, ServiceEntry};
use gpui_kit::base::v_flex;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, Entity, IntoElement, MouseDownEvent, ParentElement, Render, Window,
};
use std::rc::Rc;
use uuid::Uuid;

/// RootView（6.1）：读取 `Entity<AppState>`，组合 Header + ServiceList + Footer，
/// 处理空状态占位与引导；承载 Add/Edit Modal、删除确认与全局开关交互。
pub struct RootView {
    state: Entity<AppState>,
}

impl RootView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &App) -> Self {
        // 拦截窗口关闭：返回 false 阻止关闭，改为最小化，保持托盘常驻；
        // 与标题栏"关闭"一致，从 KWin 任务栏隐藏（托盘激活后恢复）。
        window.on_window_should_close(cx, |window, _cx| {
            window.minimize_window();
            std::thread::spawn(crate::kwin::hide_via_kwin);
            false
        });
        // gpui-fast X11 后端会在 GPU 初始化完成（异步）后写入自己的
        // WM_SIZE_HINTS（max_size 恒为纹理上限），同步写会被其覆盖。
        // 因此延迟并多次重试写入，确保最终落在后端写入之后：
        // 锁宽 400（min==max），高度 [640, 上限] 可调；此后 WM 直接拒绝
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
        Self { state }
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
                    // WM_SIZE_HINTS: 锁宽 400（min_width == max_width），
                    // 高度 [640, 16384] 可调；flags=0x30 (PMinSize|PMaxSize)
                    let _ = conn.change_property32(
                        x11rb::protocol::xproto::PropMode::REPLACE,
                        wid,
                        x11rb::protocol::xproto::AtomEnum::WM_NORMAL_HINTS,
                        x11rb::protocol::xproto::AtomEnum::WM_SIZE_HINTS,
                        &[
                            0x30u32, // flags: PMinSize|PMaxSize
                            0, 0, 0, 0, 400, 640, 400, 16384, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
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
        let title = match &editing {
            Some(e) => format!("编辑服务：{}", e.service_name),
            None => "添加服务".to_string(),
        };
        // open_dialog 的 builder 是渲染闭包，重建时会再次执行：
        // 实体必须在打开对话框之前创建，builder 仅引用已有实体句柄。
        let view = cx.new(|cx| AddEditView::new(window, cx, state, editing, title));
        // 一次性初始焦点 guard：仅在 builder 首次执行时聚焦服务名输入框
        let initial_focus = std::cell::Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                view.read(cx).name_focus_handle(cx).focus(window, cx);
            }
            dialog.close_button(false).child(view.clone())
        });
    }

    /// 删除确认（6.7）：确认后移除 + 持久化 + Toast；取消不做任何变更。
    fn confirm_delete(&self, id: Uuid, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let state2 = state.clone();
            alert
                .title("删除服务")
                .description(format!("确定删除 “{}”？此操作不可撤销。", name))
                .ok_text("确定")
                .cancel_text("取消")
                .confirm()
                .on_ok(move |_, win, app| {
                    let h = win.window_handle();
                    let result = models::mutate(
                        &state2,
                        app,
                        move |s, _| Ok(s.remove(id)),
                        move |msg, app| {
                            let _ = h.update(app, |_, w, a| {
                                w.push_notification(Notification::error(msg), a);
                            });
                        },
                    );
                    if result == Some(true) {
                        win.push_notification(Notification::success("已删除"), app);
                    }
                    true
                })
        });
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = (*self.state.read(cx)).clone();
        let state_ent = self.state.clone();

        // ServiceList（6.3 / 6.4）：每个 id 构造 CardActions
        let make_actions = |id: Uuid| -> service_card::CardActions {
            let state2 = state_ent.clone();
            let entry = state.services.iter().find(|s| s.id == id).cloned();
            service_card::CardActions {
                on_toggle_mute: {
                    let state3 = state2.clone();
                    Rc::new(move |_, win, app| {
                        let h = win.window_handle();
                        let _ = models::mutate(
                            &state3,
                            app,
                            move |s, _| s.toggle_mute(id),
                            move |msg, app| {
                                let _ = h.update(app, |_, w, a| {
                                    w.push_notification(Notification::error(msg), a);
                                });
                            },
                        );
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
            cx,
        );

        // Footer：左侧全局通知开关，右侧"新建服务"
        let footer = footer::render_footer(
            state.global_notifications_enabled,
            {
                let state3 = state_ent.clone();
                move |checked, win, app| {
                    let h = win.window_handle();
                    let _ = models::mutate(
                        &state3,
                        app,
                        move |s, _| {
                            s.global_notifications_enabled = *checked;
                            Ok(())
                        },
                        move |msg, app| {
                            let _ = h.update(app, |_, w, a| {
                                w.push_notification(Notification::error(msg), a);
                            });
                        },
                    );
                }
            },
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_add_edit(None, window, cx);
            }),
            cx,
        );

        v_flex()
            .size_full()
            .child(titlebar::render_titlebar(window, cx))
            .child(list)
            .child(footer)
            .child(statusbar::render_statusbar(cx))
    }
}
