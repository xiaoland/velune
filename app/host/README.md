# 历史合成协作原型

本 unit 是无凭据的 Rust／SQLite 协作模拟 CLI，保存固定深度委派和重启恢复的历史原型。它不链接产品 application／UniFFI，不随 Mac app 交付，也不启动真实 Harness 或模型请求。

```sh
cargo run --locked -p velune-host -- demo /tmp/velune-demo.sqlite
cargo run --locked -p velune-host -- inspect /tmp/velune-demo.sqlite
```

`demo` 要求新数据库；`inspect` 取得单写权限并执行恢复，不是只读查看工具。模型与工具结果均为固定合成输入。开发证据见 [开发说明](../../docs/development.md)。
