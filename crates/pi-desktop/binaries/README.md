# Sidecar staging (Tauri `externalBin`)

`tauri build` 在打包前要看到带 **host triple** 后缀的 `pi-app-server`：

```
pi-app-server-aarch64-apple-darwin
pi-app-server-x86_64-apple-darwin
pi-app-server-x86_64-pc-windows-msvc.exe
```

不要手拷。在仓库根：

```bash
python3 packaging/prepare_sidecar.py
# 或一把做完：python3 packaging/build-desktop.py
```

这些二进制是本机构建产物，**不进 git**。
