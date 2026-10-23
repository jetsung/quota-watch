# 配额重置提醒

一款轻量的桌面配额倒计时提醒工具，基于 [GPUI Kit](https://gpui-kit.com)（Rust）构建。为每个服务（如各类 API 配额）维护一个循环的重置周期，实时显示 `HH:MM:SS` 倒计时，并在临近重置时发送桌面通知；重置时间点过后自动滚动进入下一个周期。

## 功能特性

- **服务卡片列表**：按下次重置时间排序，每秒刷新倒计时；状态徽章一眼可辨
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
- **通知机制**：
  - 首次提醒（进入阈值时，一次）→ 周期提醒（按间隔重复）→ 重置自动滚动（不受静音与全局开关影响）
  - 全局通知开关（Footer 左下角）：一键禁用所有桌面通知
  - 通知发送失败不影响倒计时循环，仅以 Toast 提示
- **配置持久化**：自动保存到本地 JSON，应用重启后恢复；解析失败时自动备份并以空配置启动
- **原生窗口**：系统标题栏承载软件名，固定宽度 400px，初始 / 最小高度 640px（高度可向上拉伸）

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

1. 点击右下角 **“新建服务”**，填写服务名与重置间隔；
2. 选择输入模式（**下次重置时间** 或 **倒计时**）并填写重置时间点 / 剩余时长；
3. 按需调整提醒阈值与周期提醒间隔（表单内回车等同点击“保存”）；
4. 卡片右下角可 **编辑** / **删除**；勾选 **“静音重复提醒”** 仅保留首次提醒；
5. Footer 左下角开关可整体禁用桌面通知（倒计时与自动重置不受影响）。

## 命令行

除图形界面外，应用提供两个子命令：

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

## 配置文件

| 平台 | 路径 |
| --- | --- |
| Linux | `$HOME/.config/quota-notifier/services.json` |
| macOS | `~/Library/Application Support/quota-notifier/services.json` |
| Windows | `%APPDATA%\quota-notifier\services.json` |

所有时间均以 UTC 存储、以系统本地时区展示；旧版本配置中的字段（如已废弃的 `timezone`）在反序列化时会被自动忽略。

## 开发

```bash
cargo test    # 运行全部单元 / 集成测试
```

代码结构：

```
src/
├── main.rs           # 入口：窗口创建、启动告警 Toast、tick 引擎
├── models.rs         # ServiceEntry / AppState / 持久化结构与校验
├── persistence.rs    # 配置加载（损坏自动备份）与原子保存
├── notifications.rs  # 桌面通知抽象（notify-rust）与文案
├── timer_engine.rs   # 1s tick：决策（提醒/重置）与标志位管理
├── tray.rs           # 系统托盘（KSNI）与窗口激活/隐藏
├── kwin.rs           # KWin D-Bus 窗口控制（Wayland 最小化/任务栏）
└── ui/
    ├── mod.rs        # RootView：列表组合与对话框交互
    ├── titlebar.rs   # 自绘标题栏与窗口控制键
    ├── header.rs / statusbar.rs  # 顶部标题与状态栏
    ├── service_list.rs / service_card.rs  # 服务列表与卡片渲染
    ├── footer.rs     # 全局通知开关 + 新建服务
    └── modal.rs      # 新建/编辑表单（模式切换、回车保存）
assets/
├── icons/            # 应用图标（svg + png）
└── linux/
    └── quota-watch.desktop  # Freedesktop 启动器
```

## License

本项目基于 [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0) 开源，完整文本见根目录 [LICENSE](LICENSE) 文件。
