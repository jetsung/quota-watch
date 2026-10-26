# 配额重置提醒

一款轻量的桌面配额倒计时提醒工具，基于 [GPUI Kit](https://gpui-kit.com)（Rust）构建。为每个服务（如各类 API 配额）维护一个循环的重置周期，实时显示 `HH:MM:SS` 倒计时，并在临近重置时发送桌面通知；重置时间点过后自动滚动进入下一个周期。

## 功能特性

- **服务卡片列表**：按置顶与下次重置时间复合排序（置顶区按置顶时间倒序），每秒刷新倒计时；状态徽章一眼可辨
  - ★ **正常**（绿底白字）
  - ☆ **即将重置**（褐底白字，进入提醒阈值后）
  - ● **重置中**（红底白字，已过重置点，等待下一 tick 滚动）
- **添加 / 编辑服务**：
  - 服务名、重置间隔（小时，支持小数如 `5.5`）
  - 两种输入模式（可随时切换，回车即保存）：
    - **下次重置时间**：输入绝对时间点 `YYYY-MM-DD HH:MM:SS`（按系统本地时区解释）
    - **倒计时**：输入距下次重置的剩余时长（时 / 分 / 秒）
  - 提醒阈值（分钟）：距重置不足该时长时触发首次提醒
  - 周期提醒间隔（分钟）：首次提醒之后每隔 N 分钟再次提醒
  - 静音：抑制周期提醒，仅保留首次提醒
  - 服务主页 URL（可选）：配置后点击卡片标题即可在系统浏览器打开
  - **置顶**：卡片底部勾选框切换，置顶条目固定排在列表最前
- **设置面板**（顶部工具栏左侧 ⚙ 按钮）：
  - 通知开关、WebDAV 云端备份同步（上传 / 下载 / 删除云端备份，云端列表仅识别 `quota-watch` 前缀的 `.json` 备份，配置密码加密存储）
  - 配置导入 / 导出
  - 关于页（版本号、检查更新、仓库与问题反馈链接）
- **通知机制**：
  - 首次提醒（进入阈值时，一次）→ 周期提醒（按间隔重复）→ 重置自动滚动（不受静音与全局开关影响）
  - 全局通知开关（设置面板）：一键禁用所有桌面通知
  - 通知发送失败不影响倒计时循环，仅以 Toast 提示
- **配置持久化**：自动保存到本地 JSON，应用重启后恢复；解析失败时自动备份并以空配置启动
- **系统托盘常驻**：缩小 / 关闭窗口即最小化到托盘（进程继续倒计时与提醒），左键点击托盘图标恢复窗口，右键菜单可显示 / 隐藏与退出
- **原生窗口**：三平台统一使用系统标题栏（拖动与最小化 / 最大化 / 关闭由系统提供），请求尺寸 445×680px，宽度锁定、高度可向上拉伸

## 构建与运行

```bash
# 构建并运行（开发版）
cargo run

# 构建发布版
cargo build --release
```

环境要求：

- Rust 1.85+（edition 2024）
- Linux 下需具备 GPUI 所需的系统库（Vulkan / fontconfig 等 GUI 运行时依赖）

## 使用说明

1. 点击顶部工具栏 **“新建服务”**，填写服务名与重置间隔；
2. 选择输入模式（**下次重置时间** 或 **倒计时**）并填写重置时间点 / 剩余时长；
3. 按需调整提醒阈值与周期提醒间隔（表单内回车等同点击“保存”）；
4. 卡片右下角可 **编辑** / **删除**；勾选 **“静音重复提醒”** 仅保留首次提醒，勾选 **“置顶”** 可将卡片固定在列表最前；
5. 点击顶部工具栏左侧 ⚙ 按钮打开 **设置面板**：可整体禁用桌面通知（倒计时与自动重置不受影响）、配置 WebDAV 云端同步、导入 / 导出配置；
6. 缩小或关闭窗口后应用最小化到系统托盘继续运行，点击托盘图标即可恢复窗口，托盘右键菜单可退出应用。

## 命令行

除图形界面外，应用提供三个子命令：

```bash
# 查看当前版本号
quota-watch version

# 查看帮助（子命令列表与说明）
quota-watch help

# 检查并一键升级到最新版本（从 GitHub Releases 下载对应平台资产并原子替换当前可执行文件，重启后生效）
quota-watch self-update
```

- `self-update` 已是最新版本时提示并不下载；GitHub API 请求失败或无当前平台升级包时以非零退出码报错。
- 不带任何子命令（或未知参数）时正常启动图形界面。

## macOS 下载与首次运行

本项目**无 Apple Developer 账号，无法提供公证（notarized）产物**，macOS 产物均为 **ad-hoc 签名（未公证）**。ad-hoc 签名保证系统把 `QuotaWatch.app` 识别为可启动应用（macOS Sequoia 15.1 起完全未签名的 .app 会被拒绝识别，双击弹"选择应用"）。剩余的 Gatekeeper「无法验证开发者」告警只针对**带 quarantine 隔离标记的文件**（浏览器下载时自动添加），无账号无法公证绕过——因此推荐方式一的终端命令下载（不产生 quarantine，全程零弹窗）；浏览器下载则按方式二放行一次即可。

Release 提供三种 macOS 产物：

| 产物 | 形态 | 签名状态 |
| --- | --- | --- |
| `QuotaWatch-macOS-universal.app.tar.xz` | 固定名 Universal `QuotaWatch.app`，由滚动 Release `latest-macos` 承载 | ad-hoc 签名（未公证） |
| `quota-watch-v<版本>-universal.app.tar.xz` | `QuotaWatch.app`（Universal，含图标与 `Info.plist`） | ad-hoc 签名（未公证） |
| `quota-watch-v<版本>-x86_64-apple-darwin.tar.xz` / `quota-watch-v<版本>-aarch64-apple-darwin.tar.xz` | 裸二进制 tarball | ad-hoc 签名（未公证） |

### 方式一（推荐）：一条命令安装，双击零弹窗

`curl` 下载与 `tar` 解压都不会写入 quarantine 隔离标记，Gatekeeper 全程不介入：

```bash
curl -fL "https://github.com/jetsung/quota-watch/releases/download/latest-macos/QuotaWatch-macOS-universal.app.tar.xz" \
  | tar -xJ -C /Applications
```

`latest-macos` 是随每次发布自动更新的固定链接（GitHub 的 latest 语义不含 prerelease，本项目长期处于 beta 阶段，故用固定 tag 的滚动 Release 承载）。执行后即得 `/Applications/QuotaWatch.app`，双击直接运行；无管理员权限时可把目标目录换成 `~/Applications`。升级到新版本时重跑本命令覆盖即可。

> 该链接自首次接入此机制的发布（v0.2.0-beta.2 起）可用；在那之前的版本请直接从 [Releases 页](https://github.com/jetsung/quota-watch/releases) 下载后按方式二放行。

### 方式二：浏览器下载，首次运行放行一次

浏览器下载 `quota-watch-v<版本>-universal.app.tar.xz` 并解压后，首次双击会弹「无法验证开发者」告警（仅首次），两种放行路径等价，任选其一：

1. **官方放行**：右键点击 `QuotaWatch.app` → **打开** → 确认；或直接双击，在弹出的警告后进入 **系统设置 → 隐私与安全性**，点击 **仍要打开**。
2. **一行命令彻底去隔离**（推荐技术用户）：

   ```bash
   xattr -dr com.apple.quarantine /path/to/QuotaWatch.app
   ```

### 排障：双击仍弹「选择应用（Choose Application）」

若双击解压出的 `QuotaWatch.app` 弹「There is no application set to open the document / 选择应用」，说明是 **LaunchServices 数据库残留了旧版本应用的失效记录**（例如曾解压双击过签名修复之前的无签名产物），与当前产物无关——当前产物已在 macOS 26 的干净环境验证：无 quarantine 时双击直接启动，带 quarantine 时走标准「无法验证开发者」告警。依次尝试：

```bash
# 1. 去隔离并强制重新注册到 LaunchServices（多数情况到此即可）
xattr -dr com.apple.quarantine /path/to/QuotaWatch.app
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /path/to/QuotaWatch.app

# 2. 仍无效：删除该副本，换一个目录重新解压（避开 LS 对旧路径的记忆）

# 3. 最后手段：重建 LaunchServices 数据库后注销重登
sudo /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -kill -seed -r -f -all local,system,user
```

> 说明：应用内 `self-update` 只认裸二进制 tarball 资产，`*.app.tar.xz`（含固定名资产）不参与匹配，互不干扰。

## 配置文件

配置文件为各平台标准配置目录下的 `config.json`（导出与 WebDAV 上传文件仍使用 `quota-watch` 系命名）：

| 平台 | 路径 |
| --- | --- |
| Linux | `$HOME/.config/quota-watch/config.json` |
| macOS | `~/Library/Application Support/cn.gcli.quota-watch/config.json` |
| Windows | `%APPDATA%\quota-watch\config.json` |

所有时间均以 UTC 存储、以系统本地时区展示；旧版本配置中的字段（如已废弃的 `timezone`）在反序列化时会被自动忽略。

## 开发

```bash
cargo test    # 运行全部单元 / 集成测试
```

代码结构：

```
src/
├── main.rs           # 入口：子命令分发、窗口创建、启动告警 Toast、tick 引擎
├── models.rs         # ServiceEntry / AppState / 持久化结构与校验
├── persistence.rs    # 配置加载（损坏自动备份）与原子保存
├── notifications.rs  # 桌面通知抽象（notify-rust）与文案
├── timer_engine.rs   # 1s tick：决策（提醒/重置）与标志位管理
├── tray.rs           # 系统托盘（tray-icon）与窗口激活/隐藏
├── window_control.rs # 跨后端窗口可见性控制（KWin D-Bus → X11 EWMH → gpui 降级）
├── kwin.rs           # KWin D-Bus 脚本（skipTaskbar 语义的 Wayland 实现）
├── win32.rs          # Windows 专属：控制台附加、任务栏隐藏等
├── update.rs         # self-update：GitHub Releases 检查、下载与原子替换
├── webdav.rs         # WebDAV 云端备份（上传/下载/列表，配置加密）
└── ui/
    ├── mod.rs        # RootView：列表组合与对话框交互
    ├── header.rs     # 顶部工具栏：设置 / 新建服务
    ├── statusbar.rs  # 状态栏
    ├── service_list.rs / service_card.rs  # 服务列表与卡片渲染
    ├── checkbox.rs   # 自绘勾选框（✓ 字符）
    ├── settings.rs   # 设置面板（通知 / WebDAV 同步 / 导入导出 / 关于）
    ├── toast.rs      # 应用内 Toast 提示（含升级操作按钮）
    └── modal.rs      # 新建/编辑表单（模式切换、回车保存）
assets/
├── icons/            # 应用图标（svg + png）
├── linux/
│   └── quota-watch.desktop  # Freedesktop 启动器
└── debian/
    └── postrm        # Debian 卸载脚本
```

## License

本项目基于 [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0) 开源，完整文本见根目录 [LICENSE](LICENSE) 文件。

## 仓库镜像

[MyCode](https://git.jetsung.com/jetsung/quota-watch) ● [AtomGit](https://atomgit.com/jetsung/quota-watch) ● [GitHub](https://github.com/jetsung/quota-watch)
