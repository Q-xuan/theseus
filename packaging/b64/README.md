# 历史快照：pi-app v0.2 源码包（base64 分段）

这是改名前的归档文件名，不是当前 Theseus 安装包。当前发货名见主 README / `theseus_*`（`Theseus.app` / `Theseus_0.6.0_*`）。

对应提交：`0db2583110f2980ce2697d448069c81ec72e160e`

- 压缩包 SHA256：`6ed73477041847b3cc1db6d4178ec57f633df8106f80b5d2e030fadad72ca1d1`
- `pi-app-v02-src.tar.gz`：70124 字节
- 完整 base64：93500 字符（`ALL.b64`）
- 四段各 23375 字符：`PART_0.b64` … `PART_3.b64`

```bash
# 任选其一
tr -d '\n' < ALL.b64 | base64 -d > pi-app-v02-src.tar.gz
# 或
cat PART_0.b64 PART_1.b64 PART_2.b64 PART_3.b64 | tr -d '\n' | base64 -d > pi-app-v02-src.tar.gz
sha256sum pi-app-v02-src.tar.gz
# 期望：6ed73477041847b3cc1db6d4178ec57f633df8106f80b5d2e030fadad72ca1d1
```
