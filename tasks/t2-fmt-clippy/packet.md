# T2 Rust 格式与 Clippy 清理

`maint/fmt-clippy` 基于 T1 的 `9bad444`。格式化覆盖 5 个 crate 的 33 个文件。运行时、应用层、远端传输和 bindings 已修复全部 lint，包含测试目标。bindings 的消息编辑及排队摘要入口把相关参数收进 UniFFI record，其 Mac、Windows、Python 调用同步更新。

已通过的 Rust 静态门禁：

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `packages/bindings` 下 `cargo clippy -- -D warnings`，分别用默认特性、`--no-default-features`、`--no-default-features --features remote`、`--all-features`

未完成：Mac、Windows 构建尚未验证新的桌面端调用签名；尚未从实际动态库生成 Python 绑定进行核对。
