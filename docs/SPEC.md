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
├── tray.rs           # 系统托盘（KSNI）与窗口激活/隐藏
├── window_control.rs # 跨后端窗口控制：隐藏到托盘 / 恢复 / 最大化（KWin 优先，X11/EWMH 回退）
├── kwin.rs           # KWin D-Bus 脚本窗口控制（Wayland）
└── ui/
    ├── mod.rs        # RootView：布局组合、对话框、删除确认、窗口关闭拦截
    ├── titlebar.rs   # 自绘标题栏与窗口控制键
    ├── service_list.rs / service_card.rs  # 服务列表（响应式分列）与卡片
    ├── footer.rs     # 全局通知开关 + 新建服务
    ├── statusbar.rs  # 版本号 + 版权信息
    ├── modal.rs      # Add/Edit 表单（模式切换、回车保存）
    └── checkbox.rs   # 自定义勾选框（✓ 字符）
```

## 2. 命令行接口

- `--version` / `-V`：打印 `quota-watch <版本号>` 后退出，不启动 GUI（供包管理器核对）。
- 其他情况：正常启动 GUI。

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
- 所有成功变更通过 `mutate` 封装：执行变更 → 持久化 → 发射 `AppStateChanged` 事件刷新 UI；校验失败时通过回调以 error Toast 显示消息，不写入状态。

### 3.5 倒计时格式化 `format_countdown`

`HH:MM:SS`，两位小时位，负值截断为 0（如 3661s → `01:01:01`，12m34s → `00:12:34`）。

## 4. 持久化（persistence.rs）

- **路径**：`directories::ProjectDirs::from("com", "quota-watch", "quota-notifier")` 的 config 目录 + `services.json`；回退 `$HOME/.config/quota-notifier/services.json`。
- **加载 `load()`**：
  - 文件不存在 → 默认空状态，无告警。
  - 解析成功 → 返回状态。
  - 解析失败 → 原文件**重命名备份**为 `services.json.bak.<YYYYMMDD-HHMMSS>`，以空状态启动，返回告警消息。
  - 读取失败 → 空状态 + 告警消息。
  - 告警消息在 GUI 启动后以 error Toast 展示。
- **保存 `save()`**：`to_string_pretty` → 写 `<path>.tmp`（`sync_all`）→ 原子 `rename`（Windows 上先删除旧文件）。失败时保留 `.tmp` 供排查，错误由 Toast 展示。
- **持久化时机**：每次通过 `mutate` 的成功变更；tick 中发生重置滚动时异步保存。
- **序列化内容**：`PersistedState { services, global_notifications_enabled }`；旧配置中的废弃字段（如 `timezone`）反序列化时自动忽略。

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

- 固定宽度 **400px**，初始高度 **720px**（完整容纳对话框表单），最小 **640px**，无上限。
- **自绘标题栏**（Client 装饰，无系统标题栏）。
- X11 下延迟重试（10 次 × 1s）写入 `WM_SIZE_HINTS` 锁宽（min=max=400，高度 [640, 16384]），避免被 gpui-fast 后端覆盖。
- `app_id = "quota-watch"`（Wayland/KWin 托盘脚本按 resourceClass 匹配）。
- **关闭拦截**：`on_window_should_close` 阻止真正关闭，改为"最小化到托盘"（`window_control::hide_to_tray`：最小化 + 隐藏任务栏图标，KWin 优先、X11/EWMH 回退，进程常驻托盘）。

### 7.2 自绘标题栏（titlebar.rs）

高 36px，左区应用图标（内联 SVG）+ 软件名"配额重置提醒"；空白区左键拖动移动窗口（双击不最大化）；按钮区拦截按下事件防止误拖动。右区三键：

- **最小化**：`minimize_window()`。
- **最大化/还原**：`window_control::toggle_maximize` 显式设置 `_NET_WM_STATE_MAXIMIZED_*`（不依赖 WM 的 Toggle 语义——实测 xfwm4 在隐藏/恢复循环后 Toggle 状态缓存会失同步）；无 X11 句柄时回退 `zoom_window()`。
- **关闭**：等同"最小化到托盘"，调 `window_control::hide_to_tray`（与关闭拦截同一入口）；hover 红 danger 色。

### 7.3 服务列表（service_list.rs）

- 按 `next_reset_at` **升序排序**展示。
- **响应式分列**：可用宽度（视口宽 - 32px 内边距）能容纳多少张最小宽度 340px 的卡片（列间距 12px）就分几列，无上限，至少 1 列；多列时卡片等宽伸缩，手动分行组成网格。
- **空状态**：🕐 图标 + "暂无服务" + "点击右下角 '新建服务' 添加第一个服务"。

### 7.4 服务卡片（service_card.rs）

每张卡片包含：服务名（16px 粗体）、状态徽章（14px 白色粗体 + 纯色背景）、28px 粗体倒计时、"下次重置: <本地时区 YYYY-MM-DD HH:MM:SS>"、静音勾选框、编辑/删除按钮。

- **状态徽章**（按剩余时间 vs 阈值）：

| 状态 | 条件 | 徽章 | 边框 |
| --- | --- | --- | --- |
| ★ 正常 | 剩余 > 阈值 | 绿 `#088f4e` | 主题 border |
| ☆ 即将重置 | 0 < 剩余 ≤ 阈值 | 褐色 | 主题 warning，卡片背景加 8% warning |
| ● 重置中 | 剩余 ≤ 0 | 主题 danger 红 | 主题 danger |

- 重置中显示 `00:00:00`，等待下一 tick 滚动。
- **URL 交互**：配置了 `url` 的条目，卡片标题变主题色可点击，`open_url` 在系统浏览器打开；未配置则纯文本。
- **右键**：卡片任意位置右键 = 删除确认（与删除按钮等效）。
- 纯函数渲染（RenderOnce 语义），每 tick 仅倒计时文本变化。

### 7.5 Footer（footer.rs）

左侧"启用桌面通知"勾选框（全局开关，变更经 `mutate` 持久化）；右侧主色"新建服务"按钮。

### 7.6 状态栏（statusbar.rs）

左侧 `v<版本号>`；右侧 `© 2026` + 作者名"Jetsung Chan"（可点击跳转主页）。文字为主题前景色 60% 透明度。

### 7.7 勾选框（checkbox.rs）

自定义实现（gpui-kit 勾选图标无法定制为字符）：16px 圆角方框，未勾选 = 主题 input 边框；勾选 = 主色背景 + 白色粗体"✓"；点击切换并回调新值。

### 7.8 Add/Edit 表单（modal.rs）

- **对话框构造约束**：`AddEditView` 实体在 `open_dialog` **之前**创建，builder 仅引用句柄（builder 重建时不会新建实体）；初始焦点 guard（`Cell<bool>`）保证仅首次构建时聚焦服务名输入框。
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

## 10. 测试规范

各模块 `#[cfg(test)]` 内联测试，覆盖：

- `parse_interval_expr` 表达式、`next_reset_at`（整数/浮点/跨天）、倒计时格式、校验拒绝、AppState 增删改。
- 旧配置含废弃 `timezone` 字段可正常反序列化。
- 持久化 roundtrip、文件不存在返回默认、损坏文件备份分支（测试通过临时目录 + 环境变量隔离，不读真实用户配置）。
- 决策引擎单测（阈值外无动作、首次仅一次、周期间隔、静音抑制 B 不抑制 A、全局禁用抑制 A/B 不抑制 C、重置多周期对齐、完整周期 A→B→C）。
- 集成测试（加载→tick→通知→重置滚动；静音/全局开关/通知失败不中断）。
- 性能基准（60 服务 < 8ms/tick）。

运行：`cargo test`。
