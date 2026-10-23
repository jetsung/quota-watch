// Release 构建显式向 MSVC 链接器传递 /SUBSYSTEM:WINDOWS 与 /ENTRY:mainCRTStartup，
// 确保 PE 头 Subsystem == 2（GUI），不弹 Console。
// debug 构建不传，保留控制台便于排障。
// 与 src/main.rs 的 #![cfg_attr(..., windows_subsystem = "windows")] 互补——
// 属性路径在某些 MSVC 工具链版本上可能未正确传递到链接器，此处直接补上。
fn main() {
    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    {
        println!("cargo:rustc-link-arg-bins=/SUBSYSTEM:WINDOWS");
        println!("cargo:rustc-link-arg-bins=/ENTRY:mainCRTStartup");
    }
}
