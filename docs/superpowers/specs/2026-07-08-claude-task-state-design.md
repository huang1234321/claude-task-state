# Claude Code 任务状态悬浮窗 — 设计文档

- 日期：2026-07-08
- 状态：已通过 brainstorming 评审 → 待写实现计划
- 作者：huangjun1（与 Claude 协作）

## 1. 目标

做一个 Windows 桌面悬浮小应用，实时显示**多个前台交互式** Claude Code 会话的任务状态（红绿灯式）。解决"同时开多个 CC，不知道哪个在跑、哪个在等你、哪个挂了"的痛点。

参照市面上结合硬件的红绿灯小工具的原理（statusline/hooks 转发），但只做纯桌面软件。

## 2. 需求（已锁定）

| 维度 | 决定 |
|---|---|
| 目标平台 | **Windows 优先**。选 Tauri，天生跨平台，未来可扩 macOS/Linux，不锁死。 |
| 监控对象 | **前台交互式 TUI 会话**（多终端 / tmux）。后台无头会话不在 MVP。 |
| 功能 | 仅红绿灯状态指示：绿 / 黄 / 橙 / 红 / 灰（橙可并入黄）。 |
| 规模 | 2~5 个并发会话，纵向小卡片。 |
| UI 技术栈 | **Tauri v2**（Rust 后端 + WebView 前端）。 |
| 数据源 | **statusline + hooks 双源**，全部转发到应用内置的本地 HTTP 采集服务。 |

## 3. 非目标（MVP 不做，但数据通路预留）

- 任务摘要文字（最后一条 prompt / 正在执行的工具）
- $cost、上下文占用 %、模型名等指标展示
- 点击聚焦到对应终端窗口
- 声音 / 系统通知 / 停止会话
- 后台无头会话（`claude agents --json` 轮询）支持

> 采集服务会照单全收 statusline 的富字段并存在会话表里，上述能力后续都是"只改前端"的低成本扩展，不需要动数据通路。

## 4. 总体架构

```
[终端1: claude] ─┐
[终端2: claude] ──┼─> statusline 脚本 + hooks
[终端3: claude] ──┘            │
                               ▼  (同步触发, cc-forward.exe 转发)
                    POST http://127.0.0.1:7331/{statusline,hook}
                               │
                               ▼
              ┌─────────────────────────────────────┐
              │   悬浮窗应用（单进程）              │
              │  ┌──────────────┐  ┌──────────────┐ │
              │  │ 采集服务     │->│ 会话状态表   │ │
              │  │ (本地 HTTP)  │  │ (RwLock<Map>)│ │
              │  └──────────────┘  └──────┬───────┘ │
              │                           │ GET /sessions (1Hz) │
              │  ┌──────────────┐  ┌──────▼───────┐ │
              │  │ 状态机       │<-│ 置顶悬浮窗 UI│ │
              │  │ (纯函数)     │  │ (红绿灯卡片) │ │
              │  └──────────────┘  └──────────────┘ │
              └─────────────────────────────────────┘
```

采集服务**嵌入悬浮窗 app 内**（一个进程，不开独立 daemon）。app 关闭即停止采集——对个人工具完全够用，省掉一套部署。

## 5. 组件

### 5.1 `cc-forward.exe`（转发器，独立小 crate）

statusline / hooks 共用的转发二进制。是整个链路里**对延迟最敏感**的组件：statusline 由 CC 同步执行，TUI 会等它返回（防抖 300ms），所以必须极快。

- 启动目标 < 10ms（原生 Rust 二进制，**不用** PowerShell/Node——它们启动 100~300ms 会让 TUI 卡）。
- 职责：读完 stdin JSON → POST 到采集服务（超时 200ms）→ 吐一行状态文本到 stdout（仅 statusline 模式需要）→ 退出。
- 两种模式：
  - `cc-forward.exe statusline`：POST `/statusline`；stdout 输出 `"<model> | <dir>"`（或链式调用用户原有的 statusLine 命令并返回其输出，见 §8）。
  - `cc-forward.exe hook`：POST `/hook`；无需 stdout。
- 配置（端口）从 app 写入的小配置文件读，如 `%APPDATA%\cc-task-state\forwarder.json` → `{ "port": 7331 }`。
- 连不上采集服务（app 没开）时：200ms 超时即放弃，statusline 模式仍输出普通状态行，**TUI 完全不受影响**。

### 5.2 采集服务（`collector.rs`，内嵌于 Tauri 后端）

`axum` 或 `tiny_http` 起本地 HTTP 服务，持有会话状态表。

**HTTP API：**

| 端点 | 方法 | Body | 说明 |
|---|---|---|---|
| `/statusline` | POST | statusline JSON | 更新会话心跳 + 富字段；触发状态机 `StatuslineBeat` |
| `/hook` | POST | hook JSON | 按 `hook_event_name` 触发对应状态机事件 |
| `/sessions` | GET | — | 返回当前会话列表（UI 轮询用） |

**会话记录字段（`SessionRecord`）：**

```rust
struct SessionRecord {
    session_id: String,
    project: String,        // cwd basename 或 workspace.project_dir
    cwd: String,
    model: Option<String>,
    state: SessionState,
    last_event: Option<(String, Instant)>,
    last_stop_at: Option<Instant>,
    last_beat_at: Option<Instant>,
    started_at: Instant,
    error_count: u32,
    // 预留：cost、context_window、last_prompt 等（MVP 不展示）
}
```

并发用 `Arc<RwLock<HashMap<String, SessionRecord>>>`。后台 reaper 任务定期把超时会话标灰/清除。

### 5.3 状态机（`state.rs`，纯函数）★ TDD 核心

把事件序列归约成红绿灯颜色。**全部状态逻辑集中在此**，与 HTTP / UI 解耦，便于穷举单测。

**状态枚举与颜色：**

| 状态 | 颜色 | 含义 |
|---|---|---|
| `Starting` | ⚪ 灰（小） | 刚 SessionStart，还没第一个心跳 |
| `Working` | 🟢 绿 | 正在生成 / 跑工具 |
| `Waiting` | 🟡 黄 | 一轮答完，等你下一条输入 |
| `WaitingPermission` | 🟠 橙 | 等你授权某个工具（比黄更急） |
| `Error` | 🔴 红 | 最近有工具失败 |
| `Ended` | ⚪ 灰 | 会话结束 / 判定已死 |

> 橙灯可并入黄灯简化；保留为独立态成本为零、信息更准，默认保留。

**转移规则（事件 → 新状态）：**

| 当前状态 | 事件 | 新状态 | 备注 |
|---|---|---|---|
| (无) | `SessionStart` | `Starting` | 登记新会话 |
| 任意 | `UserPromptSubmit` | `Working` | 用户发了新 prompt |
| 任意 | `PreToolUse` | `Working` | 开始用工具 |
| 任意 | `StatuslineBeat`（当 `last_beat_at > last_stop_at`） | `Working` | **Stop 之后的心跳 = 新轮次开始**（核心判据）|
| `Working` | `Stop` | `Waiting` | 一轮答完 |
| `Working` | `Notification(idle_prompt)` | `Waiting` | |
| 任意 | `Notification(permission_prompt)` | `WaitingPermission` | 等授权 |
| 任意 | `PostToolUseFailure` | `Error` | 短暂亮红 |
| `Error` | `UserPromptSubmit` / `PreToolUse` / `Stop` / Stop 后心跳 | 对应态 | 自动恢复（红非永久） |
| 任意 | `SessionEnd` | `Ended` | |
| `Working`/`Waiting`/`WaitingPermission` | `HeartbeatTimeout`（>30s 无心跳） | `Ended` | 当作崩溃 / 关窗 |
| `Ended` | `SessionStart`（同 session_id） | `Starting` | 极少见复用 |

> **关键技巧**：working↔waiting 用"Stop 之后是否又来了 statusline 心跳"判定，比单纯数事件稳得多——心跳天然兜底单个 hook 事件丢失。

`state.rs` 暴露纯函数 `fn transition(rec: &SessionRecord, event: Event) -> SessionRecord`，所有副作用（时间判定、reaper）以参数/外部调度注入，保证可测。

### 5.4 悬浮窗 UI（前端 web）

- 置顶（always-on-top）、无边框、半透明小窗；可拖动；记忆位置。
- 纵向排列 2~5 张卡片：`彩色条 + 项目名 + 状态词`。
- 每秒轮询 `GET /sessions` 渲染（2~5 会话 1Hz 完全够；SSE 为可选增强，不在 MVP）。
- 系统托盘图标显示**聚合色**（任一会话红→红，否则任一黄/橙→黄，全绿→绿，全灰→灰）；双击显示/隐藏悬浮窗。
- 技术：vanilla HTML/CSS/JS（卡片量小，不引框架）；与后端用 `fetch` 通信。

## 6. 数据流

1. 终端 CC 触发 statusline / hook（同步）。
2. `cc-forward.exe` 读 stdin → POST（超时 200ms）→ 退出；statusline 模式额外输出状态行。
3. 采集服务收到 POST → 更新会话表（经 `state::transition`）。
4. UI 每秒 `GET /sessions` → 渲染卡片 + 更新托盘聚合色。

## 7. 错误处理与边界

- **app 没开**：转发器 200ms 超时放弃；statusline 模式仍输出普通状态行，TUI 不受影响。
- **端口冲突**：默认 7331，app 内可改；转发器读同一配置文件。
- **僵尸会话**：reaper 把 >30s 无心跳的标 `Ended`，更久（如 5min）才从表里清除。
- **payload 字段随 CC 版本变**：解析宽松、未知字段忽略、记日志；关键字段（`session_id`、`hook_event_name`、`cwd`、`notification_type`）缺失时跳过该事件并告警。
- **重复 / 乱序事件**：状态机以 `last_stop_at` / `last_beat_at` 时间戳为准，幂等处理。
- **settings.json 人工改动**：app 启动时校验配置是否还在，缺失则提示重装。

## 8. 配置安装（首启自动）

App 第一次运行时，把以下**合并**写入 `~/.claude/settings.json`（先备份到 `settings.json.cc-task-state.bak.<ts>`，不动用户原有内容）：

- `statusLine`：
  ```json
  { "type": "command", "command": "<install_dir>\\cc-forward.exe statusline" }
  ```
  若用户已有 `statusLine`：**链式**——转发器 POST 完后，把同一 stdin 喂给原命令，返回其 stdout。（配置里保存原命令供转发器读取。）
- `hooks`：为 `SessionStart` / `SessionEnd` / `Stop` / `UserPromptSubmit` / `PreToolUse` / `PostToolUseFailure` / `Notification` 各追加一条 matcher，`command` = `cc-forward.exe hook`。与用户已有 hooks **并存追加**，不覆盖。

提供"卸载配置"按钮：从备份还原 / 精准移除本工具写入的条目。

## 9. 测试策略

- **`state.rs`（最高优先级）**：纯函数，对每个转移、每条边界、事件丢失场景写单测（TDD）。
- **HTTP 端点**：用真实 statusline / hook JSON fixtures 做集成测试（不同 CC 版本各一份）。
- **转发器**：stdin→POST 行为、超时行为、连不上时仍输出状态行。
- **端到端**：一个"假 CC"脚本回放 fixtures 序列，断言灯色随时间正确变化（覆盖 working→waiting→ended、permission、error 恢复、心跳超时）。
- **UI**：手动验证置顶 / 拖动 / 托盘；可选快照测试卡片渲染。

## 10. 项目结构

```
claude-task-state/
├─ src-tauri/              # Rust 后端
│   ├─ src/
│   │   ├─ main.rs         # Tauri 启动、置顶窗口、托盘
│   │   ├─ collector.rs    # 本地 HTTP + 会话表 + reaper
│   │   └─ state.rs        # ★ 状态机（纯函数，重点测）
│   ├─ tests/
│   │   ├─ state_tests.rs
│   │   └─ fixtures/       # statusline.json, hook_*.json
│   └─ tauri.conf.json
├─ forwarder/              # 独立 crate → cc-forward.exe
├─ src/                    # 前端 web
│   ├─ index.html
│   ├─ main.js
│   └─ style.css
└─ docs/superpowers/specs/ # 本设计文档
```

## 11. 实现期需首要验证的两点（避免踩坑）

1. **本机 CC 版本的 hooks 事件名 + payload 字段**：尤其 `Notification` 的 `notification_type` 实际取值（`idle_prompt` / `permission_prompt` 是否就是这些字符串）、`Stop` / `PostToolUseFailure` 的字段名。文档口径与各版本会有差异，先抓真实样本再定转发器契约。
2. **statusline JSON 字段名 + 同步行为**：确认 CC 是否同步等 stdout、防抖间隔、`session_id` / `workspace` / `cost` / `context_window` 字段名，再定 `SessionRecord` 与转发器。

> 验证方法：临时写一个把 stdin 原样落盘的 statusline / hook 脚本，跑一遍真实会话，收集样本 JSON 作为 fixtures。

## 12. MVP 验收标准

- [ ] 同时开 3 个 CC 会话，悬浮窗出现 3 张卡片。
- [ ] 任一会话发 prompt → 其卡片变 🟢；答完 → 🟡。
- [ ] 触发工具授权 → 🟠；工具失败 → 🔴 并能自动恢复。
- [ ] 关掉某个终端 → 对应卡片变 ⚪ 并最终消失。
- [ ] app 没开时，CC 的 TUI 状态栏不卡、不受影响。
- [ ] 托盘聚合色正确反映"是否有会话需要关注"。
