# Sidecar is PATH `pi`

v0.8 起桌面产品路径拉起 **`pi --mode rpc`**，不再把 `theseus-app-server` 打进包。

安装 [pi](https://github.com/badlogic/pi-mono)，保证 GUI 进程也能在 PATH 上找到 `pi`（Windows 不只是你的 shell）。调试可设 `THESEUS_PI_BIN`。

这个目录只给 Tauri 占位；不要手拷二进制进 git。
