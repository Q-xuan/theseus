# Theseus

忒修斯之船：壳可以换，**append-only session event log** 才是那艘船。

这是一份**独立实现**。想法对齐 session-log / app-server 这一侧：[openai/codex](https://github.com/openai/codex) 的 app-server（Thread / Turn / Item 只读外形）、[deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) 的 SessionEvent、以及 pi-mono 的 session log 取舍。**不是** pi-mono 的 fork，也不自称 “pi”。

crate / 二进制暂时仍叫 `pi-*`（`pi-core`、`pi-app-server`、`pi-desktop`…）。改名是后面单独一刀，这一刀不重命名。

## 三条冻结原则

1. **log is truth** — 九种 `SessionEvent` 的 append-only JSONL 是唯一真相；投影、UI、派生 messages 都可以扔了重来。
2. **shell stays out of core** — 桌面 / Web / `--preview` 只投影协议，不跑 derive、不写 log、不进工具环。
3. **delete before add** — 先删后加。没有 MCP、compaction、Ask/Plan 新协议、插件体系。

最小可运行的 Rust MVP：append-only session log + stdio JSON-RPC app-server。`pi-web` 是薄客户端壳（只投影协议）。模型调用走薄的 `pi-llm` seam（OpenAI 兼容流式 `chat.completions`）。同 turn 内可跑 `read` / `write` / `edit` / `bash` 工具环；危险三项可挂一层确认门闩。未签名的 macOS / Windows 安装包挂在 `v*` 标签的 [GitHub Releases](https://github.com/Q-xuan/theseus/releases)；也可以在自己的机器上 `python3 packaging/build-desktop.py`。Linux 只做 `--preview` / `cargo test`（[`.github/workflows/linux.yml`](.github/workflows/linux.yml)）。

## Crates

名字暂不改。职责如下。

| crate | 职责 |
| --- | --- |
| `pi-protocol` | 冻结的 SessionEvent 类型 + JSON Schema + Codex 外形 |
| `pi-core` | append-only 日志（内存 + JSONL 落盘）、turn/step fencing、每 turn 硬顶 20 step、`derive_messages`、`project_item`、fork |
| `pi-llm` | OpenAI 兼容流式 `chat.completions` seam；返回文本和/或 tool_calls；key 只读环境变量 |
| `pi-tools` | 本机四工具（cwd 钉在 thread 工作区；`bash` 必有超时） |
| `pi-app-server` | stdio JSON-RPC：`thread/start`、`thread/resume`、`thread/list`、`turn/start`、`tool/approve`、`tool/reject`、事件流 |
| `pi-web` | 浏览器协议客户端（连通 / 手测）。**不是**产品终态皮 |
| `pi-desktop` | Tauri 薄壳：拉起/杀掉 `pi-app-server` sidecar（stdio）；v0.4 皮按 Codex Desktop / dsh 密度；v0.5 本机打 macOS `.app` / Windows NSIS，sidecar 打进包 |

## 事件契约

历史日志只允许这九种（硬边界，不要往 history 里加新 kind）：

1. `thread/meta`
2. `turn/start`
3. `turn/end`
4. `step/start`
5. `step/end`
6. `user/message`
7. `assistant/message`
8. `tool/call`
9. `tool/result`

`assistant/chunk` 可以出现在流式 UX（`item/agentMessage/delta`），**不得**写入 session log，也不得进入 `derive_messages`，也**不得**落盘。

## Session 落盘

唯一真相仍是九种 `SessionEvent` 的 append-only log。磁盘上就是这份 log 的 JSONL，一行一个事件，与内存同序同内容。重启后用 `Session::load_jsonl` / `Session::load_thread` **重放**文件；不另存 `messages`、派生投影、compaction 索引或 sqlite。

路径约定（stdio `pi-app-server` 与 `pi-web` Hub 相同）：

| 优先级 | 环境变量 | 目录 |
| --- | --- | --- |
| 1 | `PI_SESSIONS_DIR` | `{PI_SESSIONS_DIR}/{thread_id}.jsonl` |
| 2 | `PI_HOME` | `{PI_HOME}/sessions/{thread_id}.jsonl` |
| 3 | （默认） | `~/.pi-app/sessions/{thread_id}.jsonl` |

`thread/start` 立刻写出 `thread/meta`；之后每次成功 `append` 同步追加一行并 `fsync`。落盘失败则该事件**不进内存**，进行中的 turn 失败。`Thread.path` 为 jsonl 的绝对路径，`ephemeral` 为 `false`。

`thread/resume` 按 `threadId`（或 sessions_dir 下的 path）`load_jsonl` 进内存；之后 `turn/start` **只 append**，不改写历史。`thread/list` 扫 `*.jsonl` 的 mtime，默认最近 20 条，字段只有 `id` / `preview` / `path` / `updatedAt`。单条损坏的 jsonl **跳过**，不拖垮整个 list。无分区、搜索、置顶、索引库。

### 手测：跑一轮 turn → 杀进程 → 从 jsonl 恢复

```bash
export PI_SESSIONS_DIR=/tmp/pi-sessions   # 或不设，默认 ~/.pi-app/sessions
export PI_LLM_API_KEY="..."               # 不要写进仓库
mkdir -p "$PI_SESSIONS_DIR"

# 终端 1：stdio server（或 cargo run -p pi-web）
cargo run -q -p pi-app-server

# 终端 2：initialize + thread/start + turn/start，记下 thread.id
# 看 stderr 的 sessions: 路径，或 result.thread.path

kill %1    # 杀掉 server

# 文件应仍在，且只有九种 history 事件
cat "$PI_SESSIONS_DIR"/thr_1.jsonl

# 再启动后 thread/list 能看到，thread/resume 后继续 turn/start
# 或：
cargo test -p pi-app-server resume_on_new_server_matches_events_and_derive_then_only_appends
```

### 测试

```bash
cargo test -p pi-core
cargo test -p pi-app-server
cargo test -p pi-web
cargo test --workspace
```

每条历史事件的 JSON 形状：

```json
{ "type": "turn/start", "seq": 1, "time": 1710000000000, "data": { "turn": 1 } }
```

`seq` 等于当时的 log 下标；`time` 是 Unix 毫秒。

模型可见 ⇔ 已经 append。`derive_messages` **单一来源**：

- `user/message` → `{ "role": "user", "content": "..." }`
- `assistant/message` → `{ "role": "assistant", "content": "..." }`（**只读文本**；该事件不写 `tool_calls`，旧 jsonl 里若有也忽略）
- `tool/call` → **唯一**的模型可见 tool-call 来源；合并到**同一步** `(turn, step)` 的 assistant（没有则新建空 content assistant）；禁止跨 step 挂到上一步
- `tool/result` → `{ "role": "tool", "toolCallId": "...", "content": "..." }`
- 边界 / `thread/meta` / `assistant/chunk` → 无

Codex 外形的 Item（`item/started` / `item/completed` / `turn.items`）与 SessionEvent **同一派生函数**：`pi_core::project_item` / `project_items`。app-server 只转发，不平行拼装。

`fork` = 复制日志前缀。前缀若停在未关闭的 turn 上会被拒绝。

同 turn **硬顶 20 个 step**（`pi_core::DEFAULT_MAX_STEPS_PER_TURN`）。第 21 次 `step/start` 被围栏拒绝；loop 以干净的 `turn/end` `kind: aborted` 收口，不留下开着的 step，也不写半截 `assistant/message`。

## 工具环

`turn/start`（有 `PI_LLM_API_KEY` 或注入的 seam）路径：

1. `turn/start` + `user/message` + `step/start`
2. `derive_messages(log)` → `LlmSeam`（请求带四工具 schema）
3. 模型若要调工具：**先** append `tool/call`（history 里模型可见 call 的唯一来源；**不**把 calls 写入 `assistant/message.tool_calls` 再投影）
4. 若门闩要求确认：暂停执行，发出 `item/tool/approval/request`，等 `tool/approve` / `tool/reject`（或超时当拒绝）后再 append `tool/result`（拒绝也有 result，`isError: true`），**不**半执行写盘
5. `ToolSeam` 在 **thread 工作区** 执行（`thread/start.cwd`，否则进程 cwd）；path fence 仍在，门闩是额外一层
6. append `tool/result`（失败只标 `isError: true`，不炸 log）
7. `step/end`，若未触顶则新开 step，回到 2
8. 模型纯文本收：append 纯文本 `assistant/message`（无 `tool_calls` 字段）→ `step/end` → `turn/end` `completed`。`Ok("")` 且无 `tool_calls` **不**写空 `assistant/message`，按 seam 失败同样收口：`step/end` + `turn/end` `kind: error`

chunk 仍只走 `item/agentMessage/delta`，不入 log。无 key 时 **RPC 错误**，不写 turn。seam 失败则 `turn/end` `kind: error`，**不**写半截 `assistant/message`。

### 四工具

| 工具 | 参数 | 行为 |
| --- | --- | --- |
| `read` | `{ path, offset?, limit? }` | `path` 解析（含 `..` / symlink）后必须落在工作区根下；`offset` 为 1-based 行号；`limit` 为行数 |
| `write` | `{ path, content }` | 覆盖写入；自动创建父目录 |
| `edit` | `{ path, oldString, newString, replaceAll? }` | 默认只替换一处，多处匹配则失败；`replaceAll: true` 全替换。兼容别名 `oldText` / `newText` |
| `bash` | `{ command, timeout? }` | 本机 `sh -c`，**无沙箱**；`timeout` 为**秒**（默认 30，上限 300）。超时仍会杀掉进程组并返回 `isError` |

### 危险工具门闩

进程级，不是设置页。与 Codex 一样：危险命令默认要人点一下；`read` 永远自动。

| 环境变量 | 值 | 默认 |
| --- | --- | --- |
| `PI_TOOL_APPROVAL` | `auto`（全自动）或 `approve`（`write` / `edit` / `bash` 需确认） | `approve` |
| `PI_TOOL_APPROVAL_TIMEOUT_SECS` | 未批复秒数；到期当拒绝 | `60`（`0` = 下一条非决策 RPC 即到期，便于测） |

默认 `approve` 对齐 Codex「先问再跑」；要切到 Codex `--full-auto` 一侧：`export PI_TOOL_APPROVAL=auto`。单测里的 tool-loop 走 `auto`，避免卡在确认上。

流程（先 call，再执行）：

1. append `tool/call`
2. 若需确认：通知 `item/tool/approval/request`（tool 名 + 参数摘要 + `expiresAtMs`），**不**调用工具
3. 客户端 `tool/approve` / `tool/reject`，params `{ threadId, callId }`
4. 再 append `tool/result`（拒绝 / 超时：`isError: true`，文件未改）→ 工具环继续
5. 超时在下一条入站 JSON-RPC（或迟到的 approve/reject）上结算：当拒绝、写 result、收口 step/turn，**不**留开着的 step/turn

`thread/resume` 若该 thread 仍有未决确认，会再推一张 request。pending 只在进程内；杀进程即丢，jsonl 里可能留下未配 result 的 `tool/call`。

## 运行 stdio server

```bash
cargo run -p pi-app-server
```

一行一条 JSON-RPC 2.0（NDJSON）。与 Codex 不同，本实现**写出** `"jsonrpc":"2.0"`，但也能读缺少该字段的请求。`initialize` 必须先于其他方法（对齐 Codex）。

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"clientInfo":{"name":"demo","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","id":1,"method":"thread/start","params":{}}' \
| cargo run -q -p pi-app-server
```

`thread/start` 的 result 里有 `thread.id`（形如 `thr_1`）。恢复已有对话：

```json
{"jsonrpc":"2.0","id":3,"method":"thread/list","params":{"limit":20}}
{"jsonrpc":"2.0","id":4,"method":"thread/resume","params":{"threadId":"thr_1"}}
```

再发：

```json
{"jsonrpc":"2.0","id":2,"method":"turn/start","params":{"threadId":"thr_1","input":[{"type":"text","text":"hello"}]}}
```

stdout 会先回 `turn` 对象，再流式通知：

| 通知 | 含义 |
| --- | --- |
| `session/event` | 原始 SessionEvent（含 `turn/start`、`tool/call`、`tool/result` 等） |
| `thread/started` / `turn/started` / `turn/completed` | Codex 外形 |
| `item/started` / `item/completed` | `userMessage` / `agentMessage` / `toolCall` / `toolResult` |
| `item/agentMessage/delta` | 流式 chunk，**不**进 history |
| `item/tool/approval/request` | 危险工具待确认（Codex 形 pending approval） |
| `item/tool/approval/resolved` | `approved` / `rejected` / `timeout` |

stdin EOF 或 `shutdown` 后进程退出。

### 调用真模型

密钥只从环境变量读，不要写进仓库、命令行参数或 README。

```bash
export PI_LLM_API_KEY="..."          # 必填；不要提交这个值
export PI_LLM_BASE_URL="https://ai.aruyx.com/"   # 可选，默认即此
# export PI_LLM_MODEL="gpt-4o-mini"  # 可选

cargo run -p pi-app-server -- --demo
```

`--demo` 会 `thread/start` + `turn/start`。有 key 时 stdout 里应出现模型生成的 `assistant/message`（或工具环后再出文本）。未设置 key 时 `turn/start` 返回 `PI_LLM_API_KEY is not set`，log 仍只有 `thread/meta`。

便捷入口：

```bash
cargo run -p pi-app-server -- --demo
cargo run -p pi-app-server -- --print-schema
cargo run -p pi-web
cargo run -p pi-desktop --features gui
# Linux 无 WebKit 冒烟：cargo run -p pi-desktop -- --preview
# macOS / Windows 可装包（详见「v0.5 打包」）：
# python3 packaging/build-desktop.py
cargo test
python3 packaging/build-desktop.py --check
# 有 key 且愿意打真网时：cargo test -p pi-llm -- --ignored
```

## 薄 Web UI（协议连通，非产品终态皮）

`pi-web` 仍是浏览器**只当客户端**：不实现 tool loop、不写 session log。页面只发 `thread/start` / `thread/resume` / `thread/list` / `turn/start` / `tool/approve` / `tool/reject`，只画 Thread / Turn / Item + `item/agentMessage/delta` + **一张**确认卡。顶栏下拉是最近 N 条，点开 resume。无策略编辑器。

**这层皮不是产品长相。** 桌面壳的目标观感是 Codex Desktop / dsh web（左栏会话、紧密度、工具行、门闩卡），见下面 `pi-desktop`。`pi-web/static` 只保证协议还能在浏览器里点通。

连接取更薄一侧：`pi-web` 同进程调用 `AppServer::handle_line_sink`（与 stdio 同一 JSON-RPC），经 WebSocket 边产生边推。不另起 stdio 子进程，也不在前端重放事件。

```bash
export PI_LLM_API_KEY="..."          # 只放在跑 pi-web 的环境里；页面没有密钥框
# export PI_WEB_PORT=43127           # 可选
cargo run -p pi-web
```

打开 `http://127.0.0.1:43127`。先从「最近」下拉打开已有对话，或点「新对话」。无 key 时仍能建/恢复会话，发消息会看到服务端 RPC 错误（log 不写 turn）。有 key 时能看到流式助手文本；工具环时出现朴素的 call / result 卡片。`write` / `edit` / `bash` 在默认 `approve` 下会弹出确认卡（批准 / 拒绝）。

刻意不做：MCP、compaction、会话墙/分区/搜索、插件/设置页/策略编辑器、密钥输入、多 provider、sandbox。

冒烟：`cargo test -p pi-web`。

## 桌面壳（v0.4 皮 + v0.5 可装包）

`pi-desktop` 只做两件事：管 `pi-app-server` 子进程的生死，把 **stdio JSON-RPC** 接到一层薄客户端。壳里**没有** derive / tool loop / SessionEvent 新 kind。

`PI_LLM_API_KEY` 只从启动 App 的**用户/系统环境**（或父进程）继承给 sidecar。没有密钥框、没有 `--api-key`、不进 WebView、**没有自动更新**、**不强制商店签名**。仓库和 Release 产物里都不带密钥。

### 本机聊一轮（源码 / 已安装都一样）

系统依赖：Rust 1.83+；macOS 用系统 WebKit；Windows 需要 WebView2。默认门闩是 `approve`（`write` / `edit` / `bash` 要人点）。

源码树：

```bash
export PI_LLM_API_KEY="..."          # 只给 sidecar，不要写进仓库
# 保持默认：不要设 PI_TOOL_APPROVAL=auto
cargo build -p pi-app-server
cargo run -p pi-desktop --features gui
```

已安装：双击 `pi.app` / 开始菜单 **pi** / 便携目录里的 `pi-desktop`。窗口应直接起来。Finder 不读 `.zshrc`，双击前请把 key 写进**用户环境变量**（见下）。

在打开的窗口里验收：

1. 点左栏 **新对话**（对照 dsh `SidebarRoot` 的 New Session 条）。左栏「最近」应出现一行，选中态是浅底。
2. **流式**：发 `只回复：ping`。主列出现「回合」标记、你的气泡、流式助手。
3. **拒绝一次**：再发「用 write 在工作区写 `scratch.txt`，内容 `no`」。输入条被一张门闩卡顶掉（琥珀条 + 工具名 + 摘要）。点 **拒绝**。应出现工具失败行，文件不应存在。
4. **批准一次**：再发「用 write 在工作区写 `hello.txt`，内容 `hi`」。同一张卡点 **批准**。工作区里有 `hello.txt`。
5. **resume**：左栏预览应更新；点另一行再点回来，历史还在（`thread/resume`）。

无密钥时第 2 步会出一条可读错误条（「模型密钥没有传到 sidecar…」），log 不写 turn。

开发脚手架（需 `cargo install tauri-cli --version "^2.2"`，无需 npm）：`cd crates/pi-desktop && cargo tauri dev --features gui`。

### 从 GitHub Releases 下载（未签名）

推送已有的 `v*` 标签后，[`.github/workflows/release-desktop.yml`](.github/workflows/release-desktop.yml) 会在 **macos-latest** 和 **windows-latest** 本机 runner 上跑现有的 `python3 packaging/build-desktop.py`，把安装包挂到该标签的 [Release](https://github.com/Q-xuan/theseus/releases)。Linux 工作流仍只做 check / test，不交叉编译桌面包。

1. 打开 [Releases](https://github.com/Q-xuan/theseus/releases)，选对应标签。
2. 下载（文件名跟 `tauri.conf.json` 的 `version`，当前是 **0.5.0**，不一定等于 git 标签）：
   - macOS：`pi_0.5.0_aarch64.dmg`（`macos-latest` 现为 Apple Silicon）；或 `pi_0.5.0_aarch64.app.zip`，解出 `pi.app` 拖进「应用程序」
   - Windows：`pi_0.5.0_x64-setup.exe`（当前用户 NSIS）
3. **未签名**：
   - macOS Gatekeeper 可能拦截首次打开：右键图标 → **打开**（不要双击一次被拦就放弃）
   - Windows SmartScreen 可能提示未知发布者：**更多信息** → **仍要运行**
4. 密钥仍然**只**从用户/系统环境变量 `PI_LLM_API_KEY` 读取。没有密钥设置页，也不要把 key 写进仓库或 `.env`。

维护者：先审查再**由人**推 `v*` 标签（工作流不会创建标签）。已有标签可在 Actions 里对 `release-desktop` 做 `workflow_dispatch`，**必须**填一个已存在的 tag；不接受空 tag，以免误建 Release。

### v0.5 打包：本机安装器 / 便携包

在 **macOS 或 Windows** 上（要打哪边就在哪边编；Linux 交叉打不出能用的 `.app` / NSIS）：

```bash
cargo install tauri-cli --version "^2.2"
python3 packaging/build-desktop.py
```

| 产物 | 路径（相对仓库根） |
| --- | --- |
| macOS `.app` | `target/release/bundle/macos/pi.app` |
| macOS DMG | `target/release/bundle/dmg/pi_0.5.0_*.dmg` |
| Windows NSIS（当前用户，未签名） | `target/release/bundle/nsis/pi_0.5.0_*-setup.exe` |
| 便携（两二进制同目录） | `python3 packaging/build-desktop.py --portable` → `dist/pi-portable-<triple>/` |

包内嵌 `pi-app-server`。关窗走 `shutdown`，超时杀进程组，不留孤儿。macOS 本机 ad-hoc 签名（`signingIdentity: "-"`）；Windows `certificateThumbprint` 为空。第一次被 Gatekeeper 拦：右键 → 打开。Windows SmartScreen 可能警告未知发布者。详细步骤与 CI 发版见 [`packaging/README.md`](packaging/README.md)。

### 环境变量（安装后双击也只认这些）

| 变量 | 作用 |
| --- | --- |
| `PI_LLM_API_KEY` | sidecar 调模型；空则 `turn/start` 失败、不写 turn |
| `PI_TOOL_APPROVAL` | 默认 `approve` |
| `PI_SESSIONS_DIR` / `PI_HOME` | jsonl 位置，默认 `~/.pi-app/sessions` |
| `PI_APP_SERVER_BIN` | 调试用覆盖 sidecar 路径 |

**Windows**：设置 → 系统 → 关于 → 高级系统设置 → 环境变量 → **用户变量** 新建 `PI_LLM_API_KEY` → 完全退出 pi 再开（必要时注销，让 Explorer 重新继承）。

**macOS**：Finder 启动的 App **不**读 `~/.zshrc`。登录会话里 `launchctl setenv PI_LLM_API_KEY '...'` 后再从 Dock 打开；或直接跑 `/Applications/pi.app/Contents/MacOS/pi`（继承当前 shell）。`open -a pi` 传不进 export。不要做密钥输入框。

### 生命周期

开 App → 定位并拉起包内（或旁边的）`pi-app-server`（stdin/stdout）→ 先发 `initialize` → 再开窗口。关 App / Ctrl-C → `shutdown`，超时则杀进程组，不留孤儿。

定位顺序：`PI_APP_SERVER_BIN` → 与本可执行文件同目录（以及 `.app` 的 `Contents/MacOS` / `Resources`）的 `pi-app-server`（可带 target triple 后缀）→ 工作区 `target/debug|release`。

### Linux 冒烟（`--preview`，不挡打包）

`cargo check` / `cargo test` **不**开 `gui` feature。无 WebKit 时用同一层皮：

```bash
cargo build -p pi-app-server
cargo run -p pi-desktop -- --preview
# http://127.0.0.1:43173   可选 PI_DESKTOP_PORT
python3 packaging/build-desktop.py --check    # 核对 tauri 包配置，不交叉编译
```

只绑 127.0.0.1。stdio↔WS 是过渡胶水，不是远程控制。Linux CI **不能**产出 macOS/Windows 安装包；能绿的是上面的 `check` / `test` / `--preview`。工作流：[`.github/workflows/linux.yml`](.github/workflows/linux.yml)（保持 check / preview，不改）。桌面包只在 macOS / Windows runner 上打，见 [`.github/workflows/release-desktop.yml`](.github/workflows/release-desktop.yml)。

### 客户端皮对照（模块级，不是像素）

`crates/pi-desktop/ui` 只投影协议。密度和结构对照：

| 我们 | 对照 |
| --- | --- |
| 左栏 240 / 12px 边距 / 38×12「新对话」 | dsh `ui-sidebar` `SidebarRoot.module.css` |
| 「最近」一行预览 + 相对时间 + 选中底 + 等待琥珀点 | dsh `ui-workspace` 会话行；Codex 左栏 chats |
| 主列回合标记 / 用户气泡 / 助手正文 | dsh `ui-conversation` ChatView；Codex thread |
| 工具名芯片 + 一行摘要，点开才见全文 | dsh `ui-tool` GenericToolCard |
| 输入条被一张门闩卡顶掉：工具名 + 摘要 + 拒/批 | dsh `ApprovalPanel`（composer takeover） |

不复刻闭源 Codex.app 像素，不上第二套前端框架，不加搜索墙 / 工作区树 / 设置页。`pi-web/static` 仍是协议冒烟页，不是桌面终态。

冒烟：`cargo test -p pi-desktop`（含打包合约：`bundle.active`、内嵌 sidecar、无 updater）。

## 假设（选了更小的一侧）

- 每个 thread 对应一个 JSONL（见上）；`thread.ephemeral` 为 `false`，`path` 为该文件。不进 git、不进前端设置页。
- 对外 `Thread.createdAt` 用 Unix **秒**（Codex）；内部 `SessionEvent.time` 用 Unix **毫秒**（harness）。
- `turn.id` 为 `turn_{threadId}_{n}`；事件里的 `turn` / `step` 是从 1 起的整数。
- 不实现 `thread/fork` RPC、`turn/steer`、工具托管或审批产品（无策略页、无 per-tool ACL）。门闩只有进程级 `auto`/`approve` + 一张确认卡。`Session::fork` 只在 `pi-core`。
- 不 vet 完整 Codex / harness / pi-mono 树，只对齐上述外形与九种事件。
- `TurnEndReason` 仍用 `aborted`（未改名为冻结侧的 `rejected`）。seam 失败用 `error`；触顶用 `aborted`。
- 默认 LLM base 为 `https://ai.aruyx.com/`；单 provider，无注册表。HTTPS 走 `native-tls`（需系统 OpenSSL / `libssl-dev`）。
- `bash` 先本机、无沙箱隔离；超时是硬保证。工具 cwd 钉 thread 工作区（canonicalize 后的根）。`read` / `write` / `edit` 路径经同一校验：解析后必须落在该根下，越界只 `tool/result.isError`，不炸 turn。
