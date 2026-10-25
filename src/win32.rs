//! Windows 专属窗口兜底（`cfg(windows)` 隔离，Linux/macOS 不参与编译）。
//!
//! gpui-fast 0.1.8 的 Windows 平台实现缺少两处能力，需 Win32 兜底：
//! - `PlatformWindow::start_window_move` 未实现 → 自绘标题栏无法拖动；
//! - `zoom()` 仅 `SW_MAXIMIZE`，无还原路径 → 最大化按钮无法还原。
//! 详见 openspec 变更 `fix-windows-window-bugs` 的 1.1 探测结论。

use gpui_kit::Window;
use raw_window_handle::RawWindowHandle;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, HGDIOBJ, RGBQUAD,
};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, GWL_EXSTYLE, GetWindowLongPtrW, ICON_BIG, ICON_SMALL, ICONINFO, SW_HIDE,
    SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SendMessageW,
    SetWindowLongPtrW, SetWindowPos, ShowWindowAsync, WM_SETICON, WS_EX_TOOLWINDOW,
};

/// 从 gpui 窗口取 HWND；取不到时返回 None（调用方静默降级，不崩溃）。
/// 走 `HasWindowHandle` trait 方法（gpui-fast 同名 inherent 方法返回其私有句柄类型，不携带 HWND）。
fn hwnd(window: &Window) -> Option<HWND> {
    // 完全限定 trait 方法：同名 inherent 方法优先且返回私有句柄类型，不可用
    match raw_window_handle::HasWindowHandle::window_handle(window)
        .ok()?
        .as_raw()
    {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as isize as *mut _)),
        _ => None,
    }
}

/// 动态增删 `WS_EX_TOOLWINDOW` 扩展样式，控制窗口是否在任务栏显示按钮
/// （等价于 Linux 版 `hide_via_kwin` 的 `skipTaskbar` 语义）。
fn set_tool_window(window: &Window, tool: bool) {
    let Some(h) = hwnd(window) else {
        return;
    };
    unsafe {
        let style = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let mask = WS_EX_TOOLWINDOW.0 as isize;
        let new = if tool { style | mask } else { style & !mask };
        if new == style {
            return;
        }
        let _ = SetWindowLongPtrW(h, GWL_EXSTYLE, new);
        // 仅 SWP_FRAMECHANGED 在窗口已最小化时不足以让 Explorer 刷新任务栏按钮，
        // 需 hide/show 循环（SWP_SHOWWINDOW 保持窗口当前的最小化/正常状态）。
        let _ = ShowWindowAsync(h, SW_HIDE);
        let _ = SetWindowPos(
            h,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
        );
    }
}

/// 隐藏任务栏按钮（"关闭"最小化到托盘时调用，进程与托盘保持）。
pub fn hide_taskbar(window: &Window) {
    set_tool_window(window, true);
}

/// 恢复任务栏按钮（托盘单击 / 菜单恢复窗口时调用）。
pub fn show_taskbar(window: &Window) {
    set_tool_window(window, false);
}

/// 将窗口图标（任务栏按钮图标）设为内嵌的应用图标 PNG。
/// gpui 的 `WindowOptions.icon` 仅 X11 生效，Windows 需 `WM_SETICON`。
pub fn set_window_icon(window: &Window) {
    let Some(h) = hwnd(window) else {
        return;
    };
    let Ok(img) = image::load_from_memory(include_bytes!("../assets/icons/quota-watch-256.png"))
        .map(|i| i.to_rgba8())
    else {
        return;
    };
    let (w, hh) = (img.width() as i32, img.height() as i32);
    unsafe {
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -hh, // 负值 = top-down，与 PNG 行序一致
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            // BI_RGB 压缩不使用调色板
            bmiColors: [RGBQUAD::default(); 1],
        };
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        let Ok(color_bm) = CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            return;
        };
        {
            // RGBA → BGRA 写入（Windows DIB 字节序）
            let dst = bits as *mut u8;
            for y in 0..hh {
                for x in 0..w {
                    let s = img.get_pixel(x as u32, y as u32);
                    let d = dst.add(y as usize * (w as usize * 4) + x as usize * 4);
                    *d = s[2];
                    *d.add(1) = s[1];
                    *d.add(2) = s[0];
                    *d.add(3) = s[3];
                }
            }
        }
        // 1bpp 全零 mask：32bpp 颜色图的 alpha 通道已承载透明信息
        let mask_bm = CreateBitmap(w, hh, 1, 1, None);
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bm,
            hbmColor: color_bm,
        };
        if let Ok(icon) = CreateIconIndirect(&info) {
            let icon_ptr = LPARAM(icon.0 as isize);
            let _ = SendMessageW(h, WM_SETICON, WPARAM(ICON_BIG as usize), icon_ptr);
            let _ = SendMessageW(h, WM_SETICON, WPARAM(ICON_SMALL as usize), icon_ptr);
        }
        let _ = DeleteObject(HGDIOBJ(color_bm.0));
        let _ = DeleteObject(HGDIOBJ(mask_bm.0));
    }
}

/// `--version` 输出回退：GUI 子系统（release）无控制台时尝试附加父进程控制台，
/// 使从终端运行 `quota-watch --version` 仍能看到版本号；附加失败（如双击）则静默。
#[cfg_attr(debug_assertions, allow(dead_code))] // 仅 release 分支调用
pub fn attach_parent_console() {
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}
