//! KWin Wayland 窗口控制（D-Bus 脚本接口，kdotool 同款原理）。
//!
//! Wayland 协议本身缺少两类能力：
//! - 应用无法自主取消最小化（无 unminimize 请求），gpui 的 xdg-activation
//!   token 又因缺少有效输入 serial 通常被 KWin 拒绝；
//! - 应用无法控制窗口是否显示在任务栏（无 skip_taskbar 请求）。
//!
//! 这里统一走 KWin 的 D-Bus 脚本接口解决：
//! - [`activate_via_kwin`]：恢复最小化、显示在任务栏并置顶激活；
//! - [`hide_via_kwin`]：最小化并隐藏任务栏图标（标题栏“关闭”语义）。
//!
//! 非 KWin 环境下 qdbus 调用会失败并被静默忽略（返回 false），不影响主流程。

use std::process::Command;

const SERVICE: &str = "org.kde.KWin";

/// 执行一段 KWin JS 脚本（写入临时文件 → loadScript → run → unload）。
fn run_script(name: &str, body: &str) -> bool {
    const HEADER: &str = "\
(function() {
    var wins = (typeof workspace.windowList === 'function')
        ? workspace.windowList() : workspace.clientList();
    for (var i = 0; i < wins.length; i++) {
        var w = wins[i];
        if (w.resourceClass && w.resourceClass.indexOf('quota') !== -1) {
";
    const FOOTER: &str = "\
        }
    }
})();
";
    let path = std::env::temp_dir().join(format!("quota-watch-{name}.js"));
    if std::fs::write(&path, format!("{HEADER}{body}{FOOTER}")).is_err() {
        return false;
    }
    let qdbus = |args: &[&str]| -> Option<std::process::Output> {
        let mut c = Command::new("qdbus");
        c.arg(SERVICE).args(args);
        c.output().ok()
    };
    let _ = qdbus(&["/Scripting", "org.kde.kwin.Scripting.unloadScript", name]);
    let Some(out) = qdbus(&[
        "/Scripting",
        "org.kde.kwin.Scripting.loadScript",
        path.to_string_lossy().as_ref(),
        name,
    ]) else {
        return false;
    };
    let id: i32 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(-1);
    if id < 0 {
        return false;
    }
    let _ = qdbus(&[&format!("/Scripting/Script{id}"), "org.kde.kwin.Script.run"]);
    let _ = qdbus(&["/Scripting", "org.kde.kwin.Scripting.unloadScript", name]);
    true
}

/// 恢复窗口：取消最小化、显示任务栏图标、置顶并激活。
pub fn activate_via_kwin() -> bool {
    run_script(
        "activate",
        "\
        w.minimized = false;
        w.skipTaskbar = false;
        if (typeof workspace.activeWindow !== 'undefined') { workspace.activeWindow = w; }
        else { workspace.activeClient = w; }
",
    )
}

/// 隐藏窗口：最小化并隐藏任务栏图标（进程保持运行，托盘常驻）。
pub fn hide_via_kwin() -> bool {
    run_script(
        "hide",
        "\
        w.minimized = true;
        w.skipTaskbar = true;
",
    )
}
