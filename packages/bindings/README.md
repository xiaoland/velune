# 跨语言接入

`velune-bindings` 是共享 application unit 的 UniFFI 接入层。Swift、Kotlin 和 Python 使用同一份 Rust 元数据生成接口。领域 package 不依赖 UniFFI；此处的 `Binding*` records 与 enums 将领域契约转换为语言绑定能够表达的类型。Mac 在应用进程内加载此 unit 的动态库，不通过 Host 或 socket 调用 Rust。

`VeluneApplication` 公开配置维护、会话控制、提供商导入、认证交互与关闭操作。参数和结果具有具名类型，不提供 JSON 请求派发接口。凭据解析器路径由平台显式传入；配置只保存凭据来源描述或引用，不保存秘密值。`BindingProviderModelBinding.adapter_metadata_json` 是运行时适配器拥有的模型元数据，平台只负责保留它，不解释其结构。

一个对象串行执行应用操作。`shutdown` 在应用忙碌时返回错误并保留对象，调用方可继续读取 snapshot 或明确取消 Agent 执行；关闭成功后除再次关闭外的操作返回 `Closed`。释放语言包装对象负责释放其 Rust 引用，不代表用户发出了取消 Agent 的命令。会话状态仍通过 `snapshot` 观察，本 unit 不新增事件订阅或后台宿主。

默认启用 `local-runtime`。使用 `--no-default-features` 构建会排除本地 Agent runtime；这只表示不携带本地执行能力，不提供尚未实现的远程连接。

## 生成绑定

先构建目标平台动态库，再读取该库中的 UniFFI 元数据。以下命令使用开发构建；平台发行构建由应用构建脚本负责。

```sh
cargo build --locked -p velune-bindings
cargo run --locked -p velune-bindings --features cli --bin velune-bindgen -- \
  generate --library target/debug/libvelune_bindings.dylib \
  --language swift --language kotlin --language python \
  --out-dir target/bindings
```

Swift 生成模块名为 `VeluneBindings`，FFI 模块名为 `VeluneBindingsFFI`；Kotlin 包名为 `app.velune.bindings`。绑定与其动态库必须来自同一次契约构建，UniFFI 在加载时检查接口校验信息。生成目录属于构建产物，不提交生成代码。

当前使用 UniFFI `0.32.2` 官方 Swift、Kotlin、Python 生成器。C# 生成器属于第三方工具，尚未完成兼容性验证；不能据此宣称已有 Windows 应用接入。
