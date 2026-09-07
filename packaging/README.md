# v0.5 打包（本机可装包）

Theseus 核冻结。这一刀只解决 **装得上、每天点得开**。不改 `theseus-core` 合约，不加 MCP / compaction / 第二套前端 / 自动更新 / 商店签名。Theseus 是独立实现（crate 全部 `theseus-*`），只借用 session-log / app-server 想法，不是 pi 的翻版。

## 从 GitHub Releases 下载

推送已存在的 `v*` 标签（或在 Actions 里对**已有**标签 `workflow_dispatch`）后，[`.github/workflows/release-desktop.yml`](../.github/workflows/release-desktop.yml) 在 **macos-latest** / **windows-latest** 本机 runner 上跑下面同一条 `python3 packaging/build-desktop.py`，把产物挂到该标签的 [Release](https://github.com/Q-xuan/theseus/releases)。**不会**自动创建或推送标签。

| 平台 | Release 上的文件（当前 `tauri.conf.json` version = 0.5.0） |
| --- | --- |
| macOS（`macos-latest` ≈ Apple Silicon） | `Theseus_0.5.0_aarch64.dmg`；另附 `Theseus_0.5.0_aarch64.app.zip`（解压得 `Theseus.app`） |
| Windows（`windows-latest` ≈ x64） | `Theseus_0.5.0_x64-setup.exe`（当前用户 NSIS） |
| 便携 zip | 默认 `build-desktop.py` **不**打便携目录；本机 `python3 packaging/build-desktop.py --portable` 才有。CI 若看到 `dist/theseus-portable-*` 会顺带打成 zip |

**未签名。** 无 Apple 公证 / Developer ID，无 Windows Authenticode，无 Tauri updater。

- macOS Gatekeeper：第一次从 Finder 打开若被拦，**右键 → 打开**。
- Windows SmartScreen：可能提示未知发布者 → **更多信息** → **仍要运行**。

`THESEUS_LLM_API_KEY` 仍然**只**从用户或系统环境变量进入 sidecar。没有密钥 UI，Release / 仓库 / workflow secret 里都不要放 key。安装后怎么写环境变量见下面「环境变量」。

Linux 的 [`.github/workflows/linux.yml`](../.github/workflows/linux.yml) 只做 `--check` / `cargo test` / `cargo check`，保持不动。

## 本机构建（macOS / Windows）

系统：Rust 1.83+；macOS 用系统 WebKit；Windows 要 WebView2。再装一次 Tauri CLI：

```bash
cargo install tauri-cli --version "^2.2"
```

仓库根：

```bash
python3 packaging/build-desktop.py
```

等价手搓：

```bash
python3 packaging/prepare_sidecar.py          # 把 theseus-app-server 拷到 crates/theseus-desktop/binaries/theseus-app-server-$TRIPLE
cd crates/theseus-desktop
cargo tauri build --features gui
```

没有 `tauri-cli` 时，两个 release 二进制放同一目录也算便携包：

```bash
python3 packaging/build-desktop.py --portable
# dist/theseus-portable-<triple>/theseus-desktop  +  theseus-app-server
```

关窗仍走现有 sidecar `shutdown`（`shutdown` RPC → 超时杀进程组）。

## 产物路径

在跑 `tauri build` 的那台机器上，相对仓库根：

| 平台 | 产物 | 路径 |
| --- | --- | --- |
| macOS | 可双击的 `.app` | `target/release/bundle/macos/Theseus.app` |
| macOS | 拖进「应用程序」的 DMG | `target/release/bundle/dmg/Theseus_0.5.0_*.dmg` |
| Windows | 当前用户 NSIS 安装器（**未签名**） | `target/release/bundle/nsis/Theseus_0.5.0_*-setup.exe` |
| 任一 | 便携目录 | `dist/theseus-portable-<triple>/` |

`.app` 里 sidecar 在 `Contents/MacOS/theseus-app-server`（与 `Theseus` 同级）。NSIS / 便携则是 `theseus-app-server.exe` 紧挨主程序。`locate` 先看 `THESEUS_APP_SERVER_BIN`，再看可执行文件旁边，最后才是源码树的 `target/`。

**不**做：Apple 公证、Developer ID 强制、Windows Authenticode、商店上架、自动更新产物（`createUpdaterArtifacts: false`，无 updater 插件）、Linux AppImage。macOS 本机是 ad-hoc（`signingIdentity: "-"`）。第一次从 Finder 打开若被拦，右键 → 打开。Windows SmartScreen 可能警告。CI Release 也是同一套未签名产物。

## 安装之后怎么用

1. 双击 `Theseus.app` / 开始菜单里的 **Theseus** / 便携目录里的 `theseus-desktop`。应出窗口，**没有**密钥框。
2. 密钥只从 **用户或系统环境变量**（或从终端拉起时的父进程）传给 sidecar。
3. 关窗必须带走 `theseus-app-server`（活动监视器 / 任务管理器里不应留下孤儿）。

### 环境变量

| 变量 | 谁读 | 默认 |
| --- | --- | --- |
| `THESEUS_LLM_API_KEY` | sidecar（`theseus-llm`） | 空 → `turn/start` RPC 错，不写 turn。`PI_LLM_API_KEY` **已弃用**，仅短读兼容 |
| `THESEUS_LLM_BASE_URL` / `THESEUS_LLM_MODEL` | sidecar | 见主 README。`PI_LLM_*` **已弃用**，仅短读兼容 |
| `THESEUS_TOOL_APPROVAL` | sidecar | `approve`。`PI_TOOL_APPROVAL` **已弃用**，仅短读兼容 |
| `THESEUS_SESSIONS_DIR` / `THESEUS_HOME` | sidecar | `~/.theseus/sessions`。`PI_*` / `~/.pi-app` **已弃用**，仅短读兼容 |
| `THESEUS_APP_SERVER_BIN` | 壳（调试用） | 包内嵌的 sidecar。`PI_APP_SERVER_BIN` **已弃用**，仅短读兼容 |

不要把 key 写进仓库、命令行、WebView、plist 里的明文配置页（本项目没有设置页）。

**Windows（双击能读到 key）**

1. 设置 → 系统 → 关于 → 高级系统设置 → 环境变量
2. **用户变量** 新建 `THESEUS_LLM_API_KEY`
3. 完全退出 Theseus 再开。若仍读不到，注销一次（Explorer 只在启动时继承用户环境）

**macOS（Finder 不读 `.zshrc`）**

登录后进 GUI 会话（任选）：

```bash
launchctl setenv THESEUS_LLM_API_KEY '你的key'
# 然后从 Dock / Finder 再开 Theseus
```

要开机还在，用 `~/Library/LaunchAgents/dev.theseus.env.plist` 跑一次 `launchctl setenv`（不要把这份 plist 提交到 git）。从终端直接跑二进制也会继承当前 shell：

```bash
export THESEUS_LLM_API_KEY='...'
/Applications/Theseus.app/Contents/MacOS/Theseus
```

`open -a Theseus` **不会**把当前 shell 的 export 传进去。

## 验收（装好后点一遍）

默认门闩是 `approve`。不要设 `THESEUS_TOOL_APPROVAL=auto`。

1. **新对话**：左栏「新对话」。最近列表多一行。
2. **流式**：发 `只回复：ping`。主列有回合标记、用户气泡、流式助手。
3. **拒一次**：发「用 write 在工作区写 `scratch.txt`，内容 `no`」。输入条被门闩卡顶掉 → **拒绝**。工具失败行；文件不存在。
4. **批一次**：再发「用 write 在工作区写 `hello.txt`，内容 `hi`」→ **批准**。工作区有文件。
5. **resume**：点另一行再点回来，历史还在。

无 key 时第 2 步是一条可读错误（「模型密钥没有传到 sidecar…」），jsonl 不写 turn。

## 维护者：怎么发一版

1. 审查 `main`，**由人**打并推送标签（例如 `git tag v0.5.1 && git push origin v0.5.1`）。工作流**不会**替你打标签。
2. `push` 匹配 `v*` 的标签会启动 `release-desktop`，在 macOS / Windows 上构建并 `gh-release` 上传。
3. 干跑 / 补传：Actions → **release-desktop** → Run workflow，**Tag** 填一个**已经存在**的 `v*` 标签。没有该标签会失败（故意的，避免误建 tag）。
4. 不要把 `THESEUS_LLM_API_KEY` 配进 repository secrets；构建不需要模型密钥。

## Linux CI（这台构建机）

**不能**交叉打出可用的 `.app` / `.dmg` / NSIS（需要本机 WebKit / WebView2 / 签名工具链）。Linux 产品路径仍是 `--preview`，不挡打包脚本。桌面安装包只走上面的 macOS / Windows Release 工作流。

能绿、应绿：

```bash
python3 packaging/build-desktop.py --check
cargo test --workspace
cargo check --workspace
cargo run -p theseus-desktop -- --preview    # http://127.0.0.1:43173
```

`--portable` 在 Linux 上只会拼出无 `gui` 的两个二进制，给脚本冒烟，**不是**发货包。
