# quota-watch 功能规范

## 1. 总体架构

桌面配额倒计时提醒工具，基于 GPUI Kit（Rust，edition 2024）。

```
src/
├── main.rs           # 入口：CLI 参数、配置加载、窗口创建、tick 引擎与托盘启动
├── models.rs         # ServiceEntry / AppState / 间隔表达式解析 / 校验
├── persistence.rs    # 配置加载（损坏自动备份）与原子保存
├── timer_engine.rs   # 1s tick：决策引擎（C→A→B）与标志位管理
├── notifications.rs  # 通知服务抽象（notify-rust）与文案
├── update.rs         # GitHub Releases 版本检查、资产匹配、自升级（version/self-update 子命令 + UI 升级入口）
├── tray.rs           # 系统托盘（KSNI）与窗口激活/隐藏
├── window_control.rs # 跨后端窗口控制：隐藏到托盘 / 恢复 / 最大化（KWin 优先，X11/EWMH 回退）
├── kwin.rs           # KWin D-Bus 脚本窗口控制（Wayland）
├── win32.rs          # Windows 平台适配
└── ui/
    ├── mod.rs        # RootView：布局组合、对话框、删除确认、设置入口、窗口关闭拦截
    ├── titlebar.rs   # 自绘标题栏、窗口控制键与设置按钮
    ├── service_list.rs / service_card.rs  # 服务列表（响应式分列）与卡片
    ├── footer.rs     # 新建服务按钮（全局通知开关已迁至设置面板）
    ├── statusbar.rs  # 版本号 + 版权信息
    ├── modal.rs      # Add/Edit 表单（模式切换、回车保存）
    ├── settings.rs   # 设置对话框（桌面通知 / 备份与恢复 / 导入与导出 / 关于）
    ├── toast.rs      # 居中提示框（主窗体带蒙板 / 设置页透明）
    └── checkbox.rs   # 自定义勾选框（✓ 字符）
```

## 2. 命令行接口

- `version`：打印 `quota-watch <版本号>` 后退出，不启动 GUI（供包管理器核对）。Windows Release（GUI 子系统）无控制台时自动附加父进程控制台（`win32::attach_parent_console`）。
- `self-update`：检查 GitHub Releases 最新版本；已是最新时打印"已是最新版本 v<版本号>"并退出，不做下载；发现新版本时下载当前平台资产、解压并原子替换当前可执行文件，打印"升级成功，重启应用后生效"后退出；请求失败或找不到当前平台资产时打印失败原因并**以非零退出码退出**，不修改任何文件。进度以 `\r` 原地刷新（下载百分比 → 解压 → 替换）。
- `help`：打印用法说明后退出。
- 不带子命令（或未知参数）：正常启动 GUI。旧的 `--version` / `-V` 标志已移除（BREAKING），不再打印版本号，按未知参数处理并启动 GUI。

## 3. 数据模型（models.rs）

### 3.1 ServiceEntry（持久化单元）

| 字段 | 类型 | 约束 |
| --- | --- | --- |
| `id` | `Uuid` | 唯一 |
| `service_name` | `String` | trim 后非空 |
| `interval_hours` | `f64` | > 0 且有限；支持浮点（5.5 = 5h30m） |
| `last_reset_time` | `DateTime<Utc>` | 内部一律 UTC 存储 |
| `reminder_threshold_minutes` | `u32` | ×60 秒不得超过重置间隔 |
| `secondary_reminder_interval_minutes` | `u32` | > 0 |
| `is_muted` | `bool` | 仅抑制周期提醒 |
| `reset_config_mode` | `ResetConfigMode` | `LastReset`（默认，仅兼容旧配置）/ `NextReset` / `Countdown`（serde snake_case） |
| `url` | `Option<String>`（`#[serde(default)]`） | 必须以 `http://` 或 `https://` 开头；旧配置无此字段时为 None |
| `is_pinned` | `bool`（`#[serde(default)]`） | 置顶标记；旧配置缺省为 false |
| `pinned_at` | `Option<DateTime<Utc>>`（`#[serde(default)]`） | 最近一次置顶时间（UTC），置顶区内排序键；旧配置缺省为 None |

派生时间量：

- `next_reset_at = last_reset_time + (interval_hours × 3600).round()` 秒。
- `time_until_reset(now) = next_reset_at - now`（已过期为负）。

### 3.2 间隔表达式解析 `parse_interval_expr`

间隔输入支持**四则运算表达式**：十进制数（含小数点）、`+ - * /`、括号、空白，如 `"5"`、`"5.5"`、`"5.00 * 7"`（= 35）。非法表达式或结果非有限值（除零）返回 None → 表单报错"重置间隔必须是数字或公式"。

### 3.3 校验规则（`ServiceEntry::validate`）

依次拒绝：间隔 ≤ 0 或非有限值；阈值分钟 ×60 > 间隔秒数（"提醒阈值不能超过重置间隔"）；周期提醒间隔 == 0；服务名空白；URL 非 http/https 前缀。

### 3.4 AppState（GPUI Entity）

- 字段：`services: Vec<ServiceEntry>`、`runtime: HashMap<Uuid, RuntimeFlags>`、`global_notifications_enabled: bool`。
- `RuntimeFlags { first_alert_sent: bool, last_alert_at: Option<DateTime<Utc>> }` —— **运行时标志，不持久化，重启后重算**。
- 变更方法：
  - `add`：先 validate，拒绝重复 id，初始化默认标志位。
  - `update`：先 validate；**关键计时字段（`interval_hours` 或 `last_reset_time`）变更时自动重置该条目的标志位**。
  - `remove`：移除条目并同时清除标志位，返回是否实际删除。
  - `toggle_mute`：翻转 `is_muted`。
  - `toggle_pin`：翻转 `is_pinned`；置 true 时写入 `pinned_at = Some(Utc::now())`，置 false 时清空 `pinned_at = None`。
- 所有成功变更通过 `mutate` 封装：执行变更 → 持久化 → 发射 `AppStateChanged` 事件刷新 UI；校验失败时通过回调以 error Toast 显示消息，不写入状态。

### 3.5 倒计时格式化 `format_countdown`

`HH:MM:SS`，两位小时位，负值截断为 0（如 3661s → `01:01:01`，12m34s → `00:12:34`）。

### 3.6 WebdavConfig（云端备份配置）

字段：`server_url`（必填，http/https）、`username`、`password`、`remote_dir`（置空默认 `/quota-watch`）。随 `PersistedState.webdav_config: Option<WebdavConfig>` 持久化（见 §4），凭据仅存本地配置文件；设置表单中密码输入框为**掩码模式**（`masked(true)`），不明文展示。

## 4. 持久化（persistence.rs）

- **路径**：`directories::ProjectDirs::from("cn", "gcli", "quota-watch")` 的 config 目录 + `config.json`；回退 `$HOME/.config/quota-watch/config.json`（macOS 即 `~/Library/Application Support/cn.gcli.quota-watch/config.json`）。导出与 WebDAV 上传文件仍使用 `quota-watch` 系命名（`quota-watch_<时间戳>.json`），仅本地文件名为 `config.json`。
- **加载 `load()`**：
  - 文件不存在 → 默认空状态，无告警。
  - 解析成功 → 返回状态。
  - 解析失败 → 原文件**重命名备份**为 `config.json.bak.<YYYYMMDD-HHMMSS>`，以空状态启动，返回告警消息。
  - 读取失败 → 空状态 + 告警消息。
  - 告警消息在 GUI 启动后以 Toast 展示。
- **保存 `save()`**：`to_string_pretty` → 写 `<path>.tmp`（`sync_all`）→ 原子 `rename`（Windows 上先删除旧文件）。失败时保留 `.tmp` 供排查，错误由 Toast 展示。
- **持久化时机**：每次通过 `mutate` 的成功变更；tick 中发生重置滚动时异步保存。
- **序列化内容**：`PersistedState { services, global_notifications_enabled, webdav_config }`；`webdav_config`（服务器地址/用户名/密码/远程目录）为 `Option`，旧配置无此字段时反序列化为 None；废弃字段（如 `timezone`）自动忽略。

## 5. Tick 引擎与通知决策（timer_engine.rs）

### 5.1 引擎

- GPUI Executor 上 spawn 单任务循环，**1 秒间隔**（后台执行器定时器，不阻塞渲染线程）。
- 每 tick：`now = Utc::now()` → `run_tick` → 每次都 `notify()` 刷新倒计时文本。
- 生命周期：窗口/Entity 释放后 `update` 失败，任务自然结束，无僵尸任务。

### 5.2 决策引擎 `decide`（纯函数，每 tick 每服务至多一个动作）

优先级 **Condition C > A > B**：

1. **Condition C（重置滚动）**：`now >= next_reset_at` → 计算跨越的完整周期数 `periods = elapsed / interval_secs`（至少 1），`new_last_reset = last_reset_time + periods × interval` —— **按周期起点对齐而非 now，避免漂移，支持错过多个周期的追赶**。不受静音与全局开关影响。
2. **Condition A（首次提醒）**：剩余时间（秒）≤ 阈值（秒）且本周期未提醒过 → 发送。**不受静音影响**。全局开关关闭时**不发送但视为已消费首次提醒**（避免开启后瞬时爆发）。
3. **Condition B（周期提醒）**：已发首次提醒、非静音、全局开关开启、且 `now - last_alert_at >= secondary_reminder_interval_minutes` → 发送并更新 `last_alert_at`。

### 5.3 `run_tick` 三阶段

1. 收集决策（不可变借用）。
2. 统一应用 Condition C：推进 `last_reset_time`、清除标志位；全局开关开启时发送重置通知（静音**不**抑制重置通知）。
3. 发送 A/B 通知：成功才更新标志位（失败不标记，下 tick 重试）。

### 5.4 失败处理

- 任一通知发送失败 → 记入 `error_messages`，tick 完成后经回调以 error Toast 展示（"通知发送失败：<服务名>"），**不中断 tick**。

### 5.5 性能

测试基准：60 个服务、100 次 tick 迭代，单次 tick < 8ms。

## 6. 桌面通知（notifications.rs）

- 抽象：`NotificationService` trait（`notify(title, body)`），生产实现 `DesktopNotificationService`（notify-rust，超时 8000ms），测试用 `MockNotificationService`（计数器）。
- **文案**（正文统一附 `下次重置：<本地时区 YYYY-MM-DD HH:MM:SS>`）：

| 条件 | 标题 |
| --- | --- |
| A 首次提醒 | `[<服务名>] 将在 X 分钟后重置` |
| B 周期提醒 | `[<服务名>] 即将重置！还剩 Y 分钟`（Y 最小取 1） |
| C 重置 | `[<服务名>] 配额已重置` |

## 7. UI 功能

### 7.1 窗口（main.rs / ui/mod.rs）

- 请求宽度 **445px**、初始高度 **680px**，最小 **445×680**，无上限。
- 矮窗口（< 720px）下"新建/编辑"对话框字段区滚动、操作按钮固定可见。
- **自绘标题栏**（Client 装饰，无系统标题栏）。
- X11 下延迟重试（10 次 × 1s）写入 `WM_SIZE_HINTS` 锁宽（min=max=445，高度 [680, 16384]），避免被 gpui-fast 后端覆盖。
- `app_id = "quota-watch"`（Wayland/KWin 托盘脚本按 resourceClass 匹配）。
- **关闭拦截**：`on_window_should_close` 阻止真正关闭，改为"最小化到托盘"（`window_control::hide_to_tray`：最小化 + 隐藏任务栏图标，KWin 优先、X11/EWMH 回退，进程常驻托盘）。

### 7.2 自绘标题栏（titlebar.rs）

高 36px，左区应用图标（内联 SVG）+ 软件名"配额重置提醒"；空白区左键拖动移动窗口（双击不最大化）；按钮区拦截按下事件防止误拖动。右区按钮（图标均为内联 SVG）：设置（菜单图标，打开设置对话框，见 7.10）、最小化、最大化/还原、关闭：

- **设置**：`on_settings` 回调打开 `SettingsView` 对话框（`window.open_dialog`）。
- **最小化**：`minimize_window()`。
- **最大化/还原**：`window_control::toggle_maximize` 显式设置 `_NET_WM_STATE_MAXIMIZED_*`（不依赖 WM 的 Toggle 语义——实测 xfwm4 在隐藏/恢复循环后 Toggle 状态缓存会失同步）；无 X11 句柄时回退 `zoom_window()`。
- **关闭**：等同"最小化到托盘"，调 `window_control::hide_to_tray`（与关闭拦截同一入口）；hover 红 danger 色。

### 7.3 服务列表（service_list.rs）

- **复合排序**（`compare_entries`）：置顶条目（`is_pinned`）整体排前，置顶区内按 `pinned_at` **倒序**（后置顶者在前，`None` 因数据损坏回退为最末置顶位）；未置顶条目按 `next_reset_at` **升序**，不参与置顶区排序。
- **置顶跟随滚动**：勾选置顶成功后列表滚动位置回顶（RootView 持有 `ScrollHandle`，经 `track_scroll` 绑定列表容器；取消置顶不动滚动位置）。
- **响应式分列**：可用宽度（视口宽 - 32px 内边距）能容纳多少张最小宽度 340px 的卡片（列间距 12px）就分几列，无上限，至少 1 列；多列时卡片等宽伸缩，手动分行组成网格。
- **空状态**：🕐 图标 + "暂无服务" + "点击右下角 '新建服务' 添加第一个服务"。

### 7.4 服务卡片（service_card.rs）

每张卡片包含：服务名（18px 粗体，**置顶条目前置 📌 图钉**）、状态徽章（14px 白色粗体 + 纯色背景）、28px 粗体倒计时、"下次重置: <本地时区 YYYY-MM-DD HH:MM:SS>"、静音/置顶勾选框、编辑/删除按钮。勾选框标签字号为 13px，与同行"编辑/删除"按钮文字字号一致。

- **状态徽章**（按剩余时间 vs 阈值）：

| 状态 | 条件 | 徽章 | 边框 |
| --- | --- | --- | --- |
| ★ 正常 | 剩余 > 阈值 | 绿 `#088f4e` | 主题 border |
| ☆ 即将重置 | 0 < 剩余 ≤ 阈值 | 褐色 | 主题 warning，卡片背景加 8% warning |
| ● 重置中 | 剩余 ≤ 0 | 主题 danger 红 | 主题 danger |

- 重置中显示 `00:00:00`，等待下一 tick 滚动。
- **URL 交互**：配置了 `url` 的条目，卡片标题变主题色可点击，`open_url` 在系统浏览器打开；未配置则纯文本。
- **置顶**：卡片底部操作行"静音重复提醒"旁新增"置顶"勾选框（`pin-{id}`），经 `mutate → toggle_pin` 持久化并即时重排列表，成功 Toast"已置顶/已取消置顶 <服务名>"；编辑表单不暴露置顶字段，编辑提交沿用旧条目置顶值。
- **右键**：卡片任意位置右键 = 删除确认（与删除按钮等效）。
- 纯函数渲染（RenderOnce 语义），每 tick 仅倒计时文本变化。

### 7.5 Footer（footer.rs）

仅保留主色"新建服务"按钮（右侧）。全局通知开关已从 Footer 迁移至设置面板"桌面通知"标签（见 7.10）。

### 7.6 状态栏（statusbar.rs）

左侧 `v<版本号>`；右侧 `© 2026` + 作者名"Jetsung Chan"（可点击跳转主页）。文字为主题前景色 60% 透明度。

### 7.7 勾选框（checkbox.rs）

自定义实现（gpui-kit 勾选图标无法定制为字符）：14px 圆角方框（贴近 13px 标签文字高度），未勾选 = 前景色 45% 透明度边框（input/border 色与灰色卡片背景过于接近，视觉不直观）；勾选 = 主色背景 + 主色边框 + 白色粗体"✓"（11px）。框与文字间距 4px（`gap_1`），行高与字号相等保证框与文字垂直居中；点击切换并回调新值。标签字号默认 13px，支持传入自定义字号。

### 7.8 Add/Edit 表单（modal.rs）

- **对话框构造约束**：`AddEditView` 实体在 `open_dialog` **之前**创建，builder 仅引用句柄（builder 重建时不会新建实体）；初始焦点 guard（`Cell<bool>`）保证仅首次构建时聚焦服务名输入框。
- **标题与提示样式**：对话框标题 17px 粗体，视觉层级高于字段标签；保存成功 Toast 文字居中。
- **字段**：服务名、重置间隔（支持表达式）、URL（可选）、下次重置时间 或 时/分/秒倒计时（随模式切换）、提醒阈值（分钟）、周期提醒间隔（分钟）、静音（勾选框）。
- **默认值**：新建时间隔 `5`、阈值 `60`、周期 `5`、下次重置 = now + 5h。
- **编辑预填**：间隔 `%.2` 格式；下次重置 = 当前 `next_reset_at` 本地时区格式化；倒计时 = 距下次重置的 HH:MM:SS。
- **模式**：
  - 表单只提供 `NextReset`（默认）与 `Countdown` 两种；**旧配置中的 `LastReset` 条目打开编辑时自动按 NextReset 显示**（枚举保留该变体仅为兼容序列化）。
  - 切换模式时以当前输入推导的 `last_reset_time` 为**锚点**重新预填两套输入框（时区语义保持一致）。
- **提交**（按钮或**任意输入框内回车**）：
  1. 解析与校验，失败显示内联错误（各字段有专属错误文案）。
  2. 统一推导 UTC `last_reset_time`：NextReset = 输入的本地时区时间 − 间隔；Countdown = `now + duration` − 间隔（时长为 0 拒绝）。
  3. URL 空输入视为未设置。
  4. 编辑沿用原 id，新建生成 UUID v4；经 `mutate` 写入，成功 Toast"已添加服务"/"已更新服务"并关闭对话框。
- 时间格式：`YYYY-MM-DD HH:MM:SS`，按系统本地时区解释（歧义取 earliest）；展示统一本地时区格式化。

### 7.9 删除确认（ui/mod.rs）

确认对话框："确定删除 '<服务名>'？此操作不可撤销。" 确定 → 移除 + 持久化 + success Toast"已删除"；取消不做任何变更。

### 7.10 设置对话框（settings.rs）

标题栏设置按钮打开，对话框内左侧竖排标签列 + 右侧内容区，共四个标签：**桌面通知 / 备份与恢复 / 导入与导出 / 关于**。

- **光标约定**：页内所有可点击按钮统一 `cursor_pointer()` 手型光标（组件库默认按钮为系统箭头，经 `pointer_button` 助手覆盖）；对话框内部的"取消/确定"按钮由组件库渲染，不受此约束。
- **桌面通知**：说明文字 + "启用桌面通知"勾选框（13px 标签），变更即经 `mutate` 持久化全局开关。
- **备份与恢复**：
  - WebDAV 连接配置四字段：服务器地址、用户名、密码（**掩码输入框**，不回显明文）、远程目录；"保存连接配置"（粗体）+"测试"（small）按钮一行居中。**远程目录置空时保存/使用均回填默认值 `/quota-watch`**（项目专属子目录，避免与其它项目混在同一云端目录）。
  - "上传当前配置"（primary 整行）与"查看云端列表"（整行）两个操作按钮。
  - 云端列表：条目紧凑排布（`px_2 py_1`、行距 `gap_1`），每条目显示文件名 + 修改时间，右侧"恢复""删除"两个 **无边框文字按钮**（`xsmall` + `ghost` 样式，`10px` 字号；删除按钮带 `danger` 色调）；空列表提示"云端无配置文件"**整行居中**（12px muted）。
  - 恢复/删除均需居中确认框（标题"恢复云端配置/删除云端配置"，文件名描述用 12px 小字号防折行）。
  - **五类忙碌状态（测试/上传/列表/恢复/删除）一律以居中 Toast 提示框呈现进度与结果**（见 7.11），不在页面底部显示文字提示；操作期间对应按钮呈 loading 态并整体禁用相关操作，后台异步执行不阻塞 UI。
- **导入与导出**：`rfd::FileDialog` 原生文件选择（JSON 过滤器）。导入解析 `PersistedState` 后弹居中确认框，确认即整体覆盖（服务列表、通知开关与 WebDAV 配置）并回填设置表单；导出文件名为带时间戳的 `quota-watch_<YYYYMMDD-HHMMSS>.json`，原子写入。
- **关于**：居中堆叠布局——应用名（15px 粗体）→ 版本号 `v<当前版本>`（12px muted）→ "检查更新"按钮单独一行（胶囊圆角）→ 一行两个胶囊描边按钮"GitHub"与"问题反馈"（均带 `IconName::Github` 图标，分别 `open_url` 仓库主页与 issues/new）。图标渲染要求 `main.rs` 以 `with_assets(gpui_kit::assets::Assets)` 注册内嵌图标资源。**检查更新**发现新版本时 Toast 附加"立即升级"按钮，点击后后台执行与 `self-update` 子命令相同的升级流程（`update::self_update_with_progress`），Toast 转为**持久进度提示**（250ms 轮询刷新下载百分比 → 解压 → 替换文案），期间提供"取消"按钮中断下载（置位取消标志，后台在检查点中止，原文件不受影响）；升级成功 Toast 附加"立即重启"按钮，点击经 `setsid`（Windows 用 `DETACHED_PROCESS`）脱离当前进程组延迟拉起新实例后退出当前应用。

### 7.11 居中提示框 Toast（toast.rs）

- 全屏居中浮层（`toast_overlay`，`scrim` 参数区分两处）：`occlude()` 始终拦截下层指针事件——提示期间下层按钮不可点击、hover 不变手型/高亮/tooltip，仅提示横幅及其操作按钮可交互。**主窗体（`RootView`）传 `scrim=true`**：绘制半透明黑蒙板（alpha 0.35）压暗背景、突出提示横幅（主窗体卡片同为灰色，无蒙板时提示与条目背景混同，视觉不直观）；**设置页（`SettingsView`）传 `scrim=false`**：透明置顶，不压暗设置面板。
- **横幅卡片样式（`toast_card`）**：主题 token 渲染——`popover` 表面色背景 + `border` 1px 边框 + `popover_foreground` 文字（15px 加重 SEMIBOLD、居中），`rounded_lg` 圆角、`shadow_lg` 较强阴影（顶层浮层 elevation），内边距 `px_4 py(14px)`，支持多行文案逐行渲染；与底层窗体（服务卡片浅灰、设置面板底色）明确区分，不撞色。
- 动效：淡入 + 上浮进入，2s 停留后 0.25s 淡出移除（库内 Toast 自动关闭写死 5s，故用自定义两段定时）。
- 主窗口与设置对话框共用（`BottomToast` trait / `show_toast`）；连续提示以序号（`toast_seq`）防旧任务误清新提示。
- **升级流程 Toast 变体**（`ToastState` 的 `action_upgrade` / `action_cancel` / `action_restart` 标志位驱动）：发现新版本 → 持久 Toast + "立即升级"/"关闭"按钮；升级中 → 持久进度 Toast + "取消"按钮；升级成功 → 持久 Toast + "立即重启"/"关闭"按钮。持久 Toast 不自动关闭，由操作完成或用户关闭移除。

## 8. 系统托盘（tray.rs）

- `tray-icon` crate，Linux 走 **KSNI** 后端（D-Bus StatusNotifierItem，不依赖 GTK 事件循环）。
- 图标：内嵌 `assets/icons/quota-watch-128.png`；tooltip"配额重置提醒"。
- 事件（TrayIconEvent + MenuEvent）经 crossbeam channel 投递，后台 task 每 100ms 轮询消费。
- **左键单击**：恢复并置顶激活窗口（`window_control::restore_from_tray`：先清 SKIP_TASKBAR 再激活，X11 经 EWMH `_NET_ACTIVE_WINDOW`；Wayland 走 KWin 脚本），置可见标志。
- **右键菜单**：
  - "打开 / 隐藏"：按窗口当前可见性取反切换 —— 可见 → `window_control::hide_to_tray`（最小化到托盘，与关闭语义一致）；隐藏 → `window_control::restore_from_tray`（恢复 + 置顶 + 激活）。
  - "退出"：`app.quit()`。

## 9. 跨后端窗口控制（window_control.rs + kwin.rs）

### 9.1 通道选择（window_control.rs）

"最小化到托盘"（hide）与"从托盘恢复"（restore）各有一个统一入口，内部按以下优先级选择通道，对调用方透明：

1. **KWin D-Bus 脚本**（kwin.rs，Wayland/KWin 会话）：调用成功即完成，跳过 X11。
2. **X11/EWMH 回退**（X11 非 KWin 环境，经 `wmctrl` 子进程发送，缺失时回退 x11rb 直发）：
   - hide：最小化后向 root 发 `_NET_WM_STATE` ClientMessage **添加** `_NET_WM_STATE_SKIP_TASKBAR`；
   - restore：**先同步清除**该状态位，再经 EWMH `_NET_ACTIVE_WINDOW` 取消最小化+置顶激活。
3. 两者均不可用（非 KWin 的 Wayland）：仅 gpui 最小化/激活降级。

- 顺序关键：恢复时**必须先清除 SKIP_TASKBAR 再激活**——实测 xfwm4 下带着该状态位激活、之后再移除，会使其最大化状态缓存失同步，导致后续最大化切换紊乱或无效。
- 发送后读回属性校验，未生效则后台重试（带 epoch 守卫，旧线程在新一轮 hide/restore 开始时自动让位）。
- 实测经验：同样的 ClientMessage，独立进程（wmctrl）发送必生效，应用进程内自建 x11rb 连接发送可能被 WM 忽略；此类 WM 交互行为对构建模式敏感（debug 时序慢会触发竞态），**验证一律使用 release 构建**。

### 9.2 KWin D-Bus 脚本（kwin.rs）

Wayland 协议缺少 unminimize 与 skip_taskbar 能力，统一通过 **KWin D-Bus 脚本接口**（写临时 JS → loadScript → run → unload，匹配 `resourceClass` 含 "quota" 的窗口）：

- `activate_via_kwin`：取消最小化、显示任务栏图标、置顶并激活。
- `hide_via_kwin`：最小化并隐藏任务栏图标（进程保持运行）。
- 非 KWin 环境（无 qdbus）调用失败返回 false，静默回退到 X11/EWMH 通道。

## 10. WebDAV 云端备份（webdav.rs）

- 客户端基于 `reqwest`（blocking）+ Basic Auth（用户名密码均为空时不带认证），30s 超时，UA `quota-watch/<版本>`。
- **连接测试**：对配置的远程目录 URL 发 PROPFIND `Depth: 0`，状态成功即可用。
- **列目录**：PROPFIND `Depth: 1` 解析 multistatus XML，仅保留配置文件（`is_services_file`：`quota-watch` 前缀且以 `.json` 结尾，**不兼容历史 `services` 前缀**），按修改时间降序。
- **上传**：文件名由 `timestamped_filename()` 生成为 `quota-watch_<YYYYMMDD-HHMMSS>.json`（不重名），PUT JSON 内容。**上传前 WebDAV 密码字段加密**：`encrypt_persisted` 用内嵌密钥的 XChaCha20-Poly1305（AEAD）加密，输出 `enc:v1:<base64(nonce||密文)>`，云端备份不含明文密码（内嵌密钥防云服务商侧直接读密码，不防反编译本程序者）。
- **恢复**：GET 下载后解析 `PersistedState`，`decrypt_persisted` 解密密码（无 `enc:v1:` 前缀的历史明文备份原样兼容），再覆盖本地（含 webdav_config）——UI 层已加确认框。本地导入（7.10 导入与导出）同样兼容解密。
- **删除**：DELETE 指定文件。
- **自动建目录**：`list_files` / `upload_file` 前先对目录 URL 发 MKCOL（`ensure_directory`）；目录已存在时服务器返回 405 等状态**一律忽略**，真正的错误由后续操作暴露——避免首次使用因目录不存在报 404。
- URL 拼接对文件名做路径段编码；远程目录归一化（补首尾 `/`）有单测覆盖。

## 11. 更新检查（update.rs）

- `check_latest()`：请求 GitHub Releases `latest` API，返回 `ReleaseInfo { version, assets }`（`version` 去 `v` 前缀；`assets` 为 `Vec<Asset { name, download_url }>`，对应 API 的 `assets` 数组项 `name` + `browser_download_url`）。调试时可设环境变量 `QUOTA_WATCH_RELEASES_API` 指向本地 Mock 服务器。
- `is_newer(latest, current)`：**完整 semver 比较**。`version_key` 解析主版本段为 `(major, minor, patch)` 三级元组（去 `v` 前缀、按 `.` 分段、每段取 `-`/`+` 之前的数字，非数字记 0）；主版本段不同直接比较。**预发布段参与比较**：有预发布段（`-` 之后、`+` 之前）的版本小于同主版本段的正式版，预发布之间逐标识符比较（数字按数值、字母按字符串、数字 < 字母，标识符更多的一方更大）。如 `0.2.0-beta.2 > 0.2.0-beta.1`，`0.2.0 > 0.2.0-beta.1`，`0.3.0-alpha > 0.2.9`。
- `select_platform_asset(assets)`：按编译期 `target_os` + `target_arch` 关键词匹配资产（文件名小写需同时包含 os 关键词——macOS 为 `darwin`——与 arch 关键词，且以平台压缩包格式结尾：Linux/macOS `.tar.xz`、Windows `.zip`），返回第一个匹配项；deb/rpm/SHA256SUMS 等非压缩包资产被扩展名过滤排除。
- `self_update_with_progress(is_cancelled, on_progress)`：完整自升级流程——`check_latest()` → `is_newer()` 比较（已是最新返回 `UpdateOutcome::AlreadyUpToDate`）→ `select_platform_asset()` 匹配 → `reqwest::blocking` 下载到临时文件（600s 超时，64KB 分块读写，`Content-Length` 计算百分比）→ `extract_binary` 解压（tar.xz 取包内 `quota-watch`，zip 取 `.exe`）→ 暂存到 exe 同目录（跨文件系统 rename 会 EXDEV，先 copy 落地同文件系统）→ 设可执行权限（Unix `0o755`）→ 原子替换（Linux 直接 `rename` 覆盖运行中二进制；Windows 先 rename 旧文件为 `.old` 腾路径，失败回滚，遗留的 `.old` 在下次升级开始时清理）。`is_cancelled` 在每个阶段边界轮询，返回 true 时以 `UpdateCancelled` 中止并清理临时文件。`on_progress` 回调报告 `UpdateProgress::Downloading` / `Extracting` / `Replacing`。
- `current_exe_path()`：规范化当前可执行文件路径，剥离 Linux 升级替换后 `/proc/self/exe` 带的 ` (deleted)` 后缀。
- `UpdateOutcome`：`AlreadyUpToDate`（已是最新） / `Updated`（已替换，重启后生效）。
- 常量：`REPO_URL`（仓库主页）、`ISSUES_URL`（`issues/new` 问题反馈页）、`CURRENT_VERSION`（`CARGO_PKG_VERSION`）。
- CLI `self-update` 子命令调用 `self_update_with_progress`，进度以 `\r` 原地刷新；UI"关于"标签"检查更新"发现新版本时 Toast 附加"立即升级"按钮，触发同一升级流程（见 7.10）。
- **macOS 产物与资产匹配**：CI 产出三类 macOS 资产——`*-apple-darwin.tar.xz` 裸二进制（x86_64/aarch64）与 `*-universal.app.tar.xz`（Universal 未签名 `.app`，`CFBundleIdentifier` 为 `cn.gcli.quota-watch`，与 `persistence.rs` 应用标识一致）。本项目无 Apple Developer 账号，不提供公证产物；`.app` tarball 文件名不含 `apple-darwin` 与单 `arch` 关键词，`select_platform_asset` **不会匹配它**，自更新始终选择裸二进制 tarball，两者互不干扰（macOS 首次运行放行方式见 README）。

## 12. 测试规范

各模块 `#[cfg(test)]` 内联测试，覆盖：

- `parse_interval_expr` 表达式、`next_reset_at`（整数/浮点/跨天）、倒计时格式、校验拒绝、AppState 增删改。
- 置顶：`toggle_pin` 翻转与 `pinned_at` 写入/清空、旧 JSON 缺置顶字段反序列化为未置顶；列表复合排序（后置顶者在前、未置顶按 `next_reset_at`、取消置顶恢复原序、损坏数据 `pinned_at=None` 回退最末置顶位）。
- 旧配置含废弃 `timezone` 字段可正常反序列化。
- 持久化 roundtrip、文件不存在返回默认、损坏文件备份分支（测试通过临时目录 + 环境变量隔离，不读真实用户配置）。
- `WebdavConfig` 序列化往返、WebDAV 客户端远程目录归一化。
- 密码加密：密文 `enc:v1:` 前缀与 roundtrip（nonce 随机、不含明文）、历史明文透传、整份配置 `encrypt/decrypt_persisted` 往返、篡改密文拒绝、上传→下载→解密全链路集成测试。
- 版本比较：`is_newer` 数值段比较、`version_key` 去 `v` 前缀/后缀、预发布段比较（beta.2 > beta.1、预发布 < 正式版、主版本段优先、标识符更多者更大）。
- 资产匹配：`select_platform_asset` 当前平台匹配、无匹配返回 None、deb/rpm/SHA256SUMS 等非压缩包资产跳过。
- 解压：`extract_binary` tar.xz 解包裸二进制成功、包内无可执行文件时失败。
- 决策引擎单测（阈值外无动作、首次仅一次、周期间隔、静音抑制 B 不抑制 A、全局禁用抑制 A/B 不抑制 C、重置多周期对齐、完整周期 A→B→C）。
- 集成测试（加载→tick→通知→重置滚动；静音/全局开关/通知失败不中断）。
- 性能基准（60 服务 < 8ms/tick）。

运行：`cargo test`。
