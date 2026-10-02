# Velune 文档恢复说明

本目录保留导入前重建快照的来源说明与校验资料。`RECOVERY-NOTE.pre-import.txt`、`SHA256SUMS.pre-import.txt` 和 `recovery-manifest.pre-import.json` 对应 ZIP 中尚未进行 Velune 命名同步的 8 份文档；其中的 SHA-256 不能用于声称本仓库当前文件与原提交或原工作区逐字节一致。

命名同步后的当前文件校验记录在 `SHA256SUMS.velune-imported.txt`。它只证明本次导入提交中的文件内容，不能替代原提交的 Git 对象、历史或逐字节恢复证据。恢复快照不包含原 `.git`，也没有伪造旧 Git 历史。
