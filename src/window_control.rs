//! 跨后端窗口可见性控制：隐藏到托盘 / 从托盘恢复。
//!
//! 通道选择（对调用方透明，KWin 优先）：
//! 1. KWin D-Bus 脚本（Wayland / KWin 会话）——`kwin::hide_via_kwin` /
//!    `kwin::activate_via_kwin`，静默失败时返回 false；
//! 2. X11/EWMH 回退——经 x11rb 发 `_NET_WM_STATE` ClientMessage 增删
//!    `_NET_WM_STATE_SKIP_TASKBAR`，等价于 KWin 的 `skipTaskbar` 语义；
//! 3. 两者均不可用（如非 KWin 的 Wayland）——仅 gpui 最小化/激活降级。

use gpui_kit::Window;

/// 隐藏到托盘：最小化窗口并从任务栏隐藏图标（进程与托盘常驻）。
pub fn hide_to_tray(window: &Window) {
    window.minimize_window();
    // Windows：对齐 Linux skipTaskbar 语义，关闭/隐藏时隐藏任务栏按钮
    #[cfg(target_os = "windows")]
    crate::win32::hide_taskbar(window);
    #[cfg(all(target_os = "linux", not(target_os = "android")))]
    {
        if crate::kwin::hide_via_kwin() {
            // KWin 接管完成（脚本内已含最小化与 skipTaskbar），跳过 X11 EWMH
            return;
        }
        if let Some(window_id) = x11_window_id(window) {
            // EWMH：向 root 发 _NET_WM_STATE ClientMessage（action=1 Add，
            // prop1=SKIP_TASKBAR，prop2=0，source=1），由 WM 更新窗口状态。
            // 实测部分 WM（xfwm4）在最小化与新连接事件并发时会丢失刚设置的
            // 状态位，故发送后校验属性、未生效则后台重试。
            let epoch = skip_epoch_next();
            ensure_skip_taskbar(window_id, true, epoch);
        }
    }
}

/// SKIP_TASKBAR 状态代次：hide/restore 每次调用都自增，
/// 后台重试线程发现代次已变（新一轮操作开始）即自行退出，防止旧线程
/// 在恢复后又把状态位加回去。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn skip_epoch() -> &'static std::sync::atomic::AtomicU64 {
    use std::sync::atomic::AtomicU64;
    static EPOCH: AtomicU64 = AtomicU64::new(0);
    &EPOCH
}

/// 自增并返回新代次。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn skip_epoch_next() -> u64 {
    use std::sync::atomic::Ordering;
    skip_epoch().fetch_add(1, Ordering::SeqCst) + 1
}

/// 后台校验并重试，直到 SKIP_TASKBAR 达到期望状态、代次变更或重试耗尽。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn ensure_skip_taskbar(window_id: u32, add: bool, epoch: u64) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        for _ in 0..8 {
            if net_wm_state_has_skip(window_id) == add {
                return;
            }
            set_skip_taskbar_once(window_id, add);
            std::thread::sleep(std::time::Duration::from_millis(125));
            // 代次已变：新一轮 hide/restore 开始，本线程立即让位
            if skip_epoch().load(Ordering::SeqCst) != epoch {
                return;
            }
        }
    });
}

/// 同步清除 SKIP_TASKBAR（最多 3 次尝试，短暂阻塞调用方）。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn clear_skip_taskbar_sync(window_id: u32) {
    for _ in 0..3 {
        if !net_wm_state_has_skip(window_id) {
            return;
        }
        set_skip_taskbar_once(window_id, false);
        if !net_wm_state_has_skip(window_id) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

/// 单次设置（add=true）或清除（add=false）SKIP_TASKBAR。
///
/// 实测：同样的 ClientMessage，独立进程（wmctrl）发送必生效，而应用进程内
/// 自建 x11rb 连接发送会被部分 WM（xfwm4）忽略，故优先经 wmctrl 子进程发送。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn set_skip_taskbar_once(window_id: u32, add: bool) {
    let action = if add { "add" } else { "remove" };
    let ok = std::process::Command::new("wmctrl")
        .args([
            "-i",
            "-r",
            &format!("{window_id:#x}"),
            "-b",
            &format!("{action},skip_taskbar"),
        ])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        // wmctrl 不可用时回退 x11rb 直发
        let _ = send_net_wm_state_skip_taskbar(window_id, add);
    }
}

/// 读取窗口 `_NET_WM_STATE` 属性，判断是否包含 SKIP_TASKBAR 位。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn net_wm_state_has_skip(window_id: u32) -> bool {
    net_wm_state_has_atom(window_id, b"_NET_WM_STATE_SKIP_TASKBAR")
}

/// 读取窗口 `_NET_WM_STATE` 属性，判断是否包含指定原子。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn net_wm_state_has_atom(window_id: u32, atom_name: &[u8]) -> bool {
    use x11rb::protocol::xproto::ConnectionExt as _;
    let Ok((conn, _screen)) = x11rb::connect(None) else {
        return false;
    };
    let Some(net_wm_state) = conn
        .intern_atom(false, b"_NET_WM_STATE")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .filter(|a| *a != 0)
    else {
        return false;
    };
    let Some(target) = conn
        .intern_atom(false, atom_name)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .filter(|a| *a != 0)
    else {
        return false;
    };
    conn.get_property(
        false,
        window_id,
        net_wm_state,
        x11rb::protocol::xproto::AtomEnum::ATOM,
        0,
        64,
    )
    .ok()
    .and_then(|c| c.reply().ok())
    .map(|r| {
        r.value
            .as_chunks::<4>()
            .0
            .iter()
            .any(|chunk| u32::from_ne_bytes(*chunk) == target)
    })
    .unwrap_or(false)
}

/// 从托盘恢复：取消最小化、清除任务栏隐藏、置顶激活。
pub fn restore_from_tray(window: &Window) {
    // Windows：恢复时重新显示任务栏按钮（与 hide_to_tray 的 hide_taskbar 对应）
    #[cfg(target_os = "windows")]
    crate::win32::show_taskbar(window);
    #[cfg(all(target_os = "linux", not(target_os = "android")))]
    {
        if crate::kwin::activate_via_kwin() {
            // KWin 接管完成，跳过 X11 EWMH
            window.activate_window();
            window.request_attention();
            return;
        }
        if let Some(window_id) = x11_window_id(window) {
            // 顺序关键：必须先取消最小化/激活（窗口回到 Normal），再清除 SKIP_TASKBAR。
            // 实测 XFCE/xfwm4 下面板任务栏在"窗口仍处 iconic 时移除 skip_taskbar"
            // 的组合下会出现任务栏按钮重复创建（每次恢复循环按钮数翻倍）的缓存
            // 失同步；且旧版"清 skip 前置"的理由（xfwm4 最大化 Toggle 缓存失同步）
            // 已随最大化交还系统标题栏（toggle_maximize 退役）不再成立。
            let epoch = skip_epoch_next();
            x11_restore(window_id);
            clear_skip_taskbar_sync(window_id);
            // 兜底：若同步清除未生效，继续后台校验重试（epoch 已更新，
            // 旧 hide 重试线程会让位）
            ensure_skip_taskbar(window_id, false, epoch);
        }
    }
    window.activate_window();
    window.request_attention();
}

/// 从窗口句柄提取 X11 window id（Xlib / Xcb 两种后端）。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn x11_window_id(window: &Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // gpui Window 有同名固有方法（返回 AnyWindowHandle），这里显式走
    // raw_window_handle 的 trait 方法拿原始 X11 句柄
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Xlib(h) => {
            let id = h.window as u32;
            (id != 0).then_some(id)
        }
        RawWindowHandle::Xcb(h) => {
            let id = h.window.get();
            (id != 0).then_some(id)
        }
        _ => None,
    }
}

/// 向 root 发送 `_NET_WM_STATE` ClientMessage，增（add=true）删（add=false）
/// `_NET_WM_STATE_SKIP_TASKBAR` 状态位。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn send_net_wm_state_skip_taskbar(window_id: u32, add: bool) -> Result<(), String> {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::ConnectionExt as _;
    let Ok((conn, screen)) = x11rb::connect(None) else {
        return Err("x11rb::connect failed".into());
    };
    let root = conn.setup().roots.get(screen).map(|r| r.root).unwrap_or(0);
    let Some(net_wm_state) = conn
        .intern_atom(false, b"_NET_WM_STATE")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .filter(|a| *a != 0)
    else {
        return Err("intern _NET_WM_STATE failed".into());
    };
    let Some(skip_taskbar) = conn
        .intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .filter(|a| *a != 0)
    else {
        return Err("intern _NET_WM_STATE_SKIP_TASKBAR failed".into());
    };
    // action: 0=Remove, 1=Add；data[1]/data[2] 为两个属性原子，data[3] source=1（应用）
    let action = if add { 1u32 } else { 0u32 };
    let data: [u32; 5] = [action, skip_taskbar, 0, 1, 0];
    let event = x11rb::protocol::xproto::ClientMessageEvent::new(
        32,
        window_id,
        net_wm_state,
        x11rb::protocol::xproto::ClientMessageData::from(data),
    );
    if let Err(e) = x11rb::protocol::xproto::send_event(
        &conn,
        false,
        root,
        x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT
            | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    ) {
        return Err(format!("send_event failed: {e}"));
    }
    if let Err(e) = conn.flush() {
        return Err(format!("flush failed: {e}"));
    }
    Ok(())
}

/// X11/EWMH 恢复：取消最小化并激活窗口。
///
/// 首选经 wmctrl 子进程发送 EWMH `_NET_ACTIVE_WINDOW`（独立进程发送，
/// WM 必响应，且激活自带取消最小化+置顶+聚焦）；wmctrl 不可用时回退到
/// 旧方案（直接写 `WM_STATE` Normal、map、置顶、`WM_CHANGE_STATE`）——
/// 注意该回退直接改写客户端不应写的 `WM_STATE`，部分 WM（xfwm4）下可能
/// 使其内部状态缓存失同步。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn x11_restore(window_id: u32) {
    let ok = std::process::Command::new("wmctrl")
        .args(["-i", "-a", &format!("{window_id:#x}")])
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        return;
    }
    x11_restore_legacy(window_id);
}

/// 旧版恢复方案：直接写 `WM_STATE`、map、置顶、`WM_CHANGE_STATE`
/// （迁移自 tray.rs，仅作 wmctrl 不可用时的回退）。
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn x11_restore_legacy(window_id: u32) {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::ConnectionExt as _;
    let Ok((conn, screen)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots.get(screen).map(|r| r.root).unwrap_or(0);
    let wm_state = conn
        .intern_atom(false, b"WM_STATE")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(0);
    if wm_state != 0 {
        let _ = x11rb::wrapper::ConnectionExt::change_property32(
            &conn,
            x11rb::protocol::xproto::PropMode::REPLACE,
            window_id,
            wm_state,
            x11rb::protocol::xproto::AtomEnum::ATOM,
            &[1u32, 0u32], // NormalState = 1
        );
        let _ = x11rb::protocol::xproto::map_window(&conn, window_id);
        // 置顶到所有窗口之上（XRaiseWindow 等价）
        let _ = x11rb::protocol::xproto::configure_window(
            &conn,
            window_id,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
        );
        let _ = conn.flush();
    }
    if let Some(reply) = conn
        .intern_atom(false, b"WM_CHANGE_STATE")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
    {
        let data: [u32; 5] = [1, 0, 0, 0, 0]; // NormalState = 1
        let event = x11rb::protocol::xproto::ClientMessageEvent::new(
            32,
            window_id,
            reply,
            x11rb::protocol::xproto::ClientMessageData::from(data),
        );
        let _ = x11rb::protocol::xproto::send_event(
            &conn,
            false,
            root,
            x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT
                | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        );
        let _ = conn.flush();
    }
    // SKIP_TASKBAR 的清除已前置到 restore_from_tray（顺序关键，见该处注释）
}
