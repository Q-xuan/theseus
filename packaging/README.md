# v0.8 打包（本机可装包）

桌面产品路径是 **`pi --mode rpc`**。安装包只带 Theseus 壳；`pi` 由用户放进 PATH。不改 `theseus-core` 九种事件，不加 MCP / compaction / 第二套前端 / 自动更新 / 商店签名。

## 从 GitHub Releases 下载

推送已存在的 `v*` 标签（或在 Actions 里对**已有**标签 `workflow_dispatch`）后，[`.github/workflows/release-desktop.yml`](../.github/workflows/release-desktop.yml) 在 **macos-latest** / **windows-latest** 本机 runner 上跑 `python3 packaging/build-desktop.py`，把产物挂到该标签的 [Release](https://github.com/Q-xuan/theseus/releases)。**不会**自动创建或推送标签。

| 平台 | Release 上的文件（当前 `tauri.conf.json` version = 0.8.0） |
| --- | --- |
| macOS（`macos-latest` ≈ Apple Silicon） | `Theseus_0.8.0_aarch64.dmg`；另附 `Theseus_0.8.0_aarch64.app.zip` |
| Windows（`windows-latest` ≈ x64） | `Theseus_0.8.0_x64-setup.exe`（当前用户 NSIS） |

**未签名。** 无 Apple 公证 / Developer ID，无 Windows Authenticode，无 Tauri updater。

- macOS Gatekeeper：第一次从 Finder 打开若被拦，**右键 → 打开**。
- Windows SmartScreen：可能提示未知发布者 → **更多信息** → **仍要运行**。

先装 [pi](https://github.com/badlogic/pi-mono) 并加入 PATH，再双击 Theseus。密钥只走 pi。

Linux 的 [`.github/workflows/linux.yml`](../.github/workflows/linux.yml) 只做 `--check` / `cargo test` / `cargo check`。

## 本机构建（macOS / Windows）

```bash
cargo install tauri-cli --version "^2.2"
python3 packaging/build-desktop.py
```

| 平台 | 产物 | 路径 |
| --- | --- |
| macOS | `.app` | `target/release/bundle/macos/Theseus.app` |
| macOS | DMG | `target/release/bundle/dmg/Theseus_0.8.0_*.dmg` |
| Windows | NSIS `Theseus_*_x64-setup.exe` | `target/release/bundle/nsis/Theseus_0.8.0_*-setup.exe` |

包内**不**嵌 `theseus-app-server`。Release GUI：windows subsystem + sidecar `CREATE_NO_WINDOW`，无黑控制台。

## 验收

1. PATH 上有 `pi`（Windows 用户 PATH，不只是当前 shell）。
2. 双击 Theseus，窗口起来，无黑控制台。
3. 新对话 → 流式 → 停止后出现「已停止」。
4. 左栏标题 + 相对时间，点开 resume；刻度可跳。

## 维护者：怎么发一版

1. 审查 `main`，**由人**打并推送标签。工作流**不会**替你打标签。
2. `push` 匹配 `v*` 的标签会启动 `release-desktop`。
3. 不要把模型密钥配进 repository secrets。
