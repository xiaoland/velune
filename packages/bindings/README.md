# 跨语言接入

`velune-bindings` 是共享 application unit 的 UniFFI 接入层。Swift、Kotlin 和 Python 使用同一份 Rust 元数据生成接口。领域 package 不依赖 UniFFI；此处的 `Binding*` records 与 enums 将领域契约转换为语言绑定能够表达的类型。Mac 在应用进程内加载此 unit 的动态库，不通过 Host 或 socket 调用 Rust。

`VeluneApplication` 公开配置维护、会话控制、提供商导入、认证交互与关闭操作，参数和结果具有具名类型。提供商拥有模型条目；`BindingProviderDraft` 与 `BindingAuthenticationEdit` 共同保存普通字段和认证编辑，后者明确区分保留、更新 API key 和清除。列表与 snapshot 只返回非秘密认证描述，`read_provider_api_key` 显式读取一个提供商的已保存 key。认证检查和登录以网关／提供商 ID 定位，不存在独立认证资源 CRUD。`BindingProviderModel.adapter_metadata_json` 由运行时 adapter 拥有，平台只保留它，不解释其结构。

一个对象串行执行应用操作。`shutdown` 在应用忙碌时返回错误并保留对象，调用方可继续读取 snapshot 或明确取消 Agent 执行；关闭成功后除再次关闭外的操作返回 `Closed` 类型的诊断。释放语言包装对象负责释放其 Rust 引用，不代表用户发出了取消 Agent 的命令。会话状态仍通过 `snapshot` 观察，本 unit 不新增事件订阅或后台宿主。

本 unit 在 `home_directory/logs` 装配对象专属的 tracing subscriber，不占用进程全局 subscriber。操作失败的 `BindingError.Diagnostic` 保留分类、白名单 code/phase、说明和 operation ID，平台使用该 ID 关联日志；领域包不依赖这一日志输出或 UniFFI。日志安全字段、轮转和后续 layer 扩展见 [开发说明](../../docs/development.md#本地诊断)。

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

模型条目的 `recordKey` 自动生成并隐藏；`providerModelId` 是提供商规定的可编辑 API 标识。运行时选择与 snapshot 引用稳定 `modelRecordKey`，导入选择只包含 `candidateKeys`。能力中的 `None` 表示未知，空推理等级列表表示明确不支持。`BindingModelTemplate` 复制填写参数，不形成调用依赖。旧全局模型、关联映射和中央认证接口已删除，不保留兼容入口。

运行时描述携带 `family_id`、精确 variant ID 与 `version_regex`；schema 6 配置通过应用用例持久化。当前 variant 为 Pi 1.0.2、Codex 0.159.3 与 DSH 0.2.0-rc.2，绑定不根据 family 自行选择协议。`BindingRuntimeInteraction` 与 `BindingRuntimeInteractionReply` 表达审批选项、问题／答案和取消；回复关联当前交互 ID，不通过字符串命令或隐式批准。原生 snapshot 历史由 huihua／Harness adapter 投影，绑定不暴露来源 JSON 或实现会话存储。

运行时 descriptor 的 `supported_protocols` 为具名协议枚举列表；平台按精确 variant 消费，不按 family 自行维护协议矩阵。此公开描述增补不改变 schema 6 配置。

`fetch_public_model_catalog` 返回具名目录候选，而不是配置模型或来源 JSON。它是有界同步调用，平台放在工作队列；保存选中模板仍使用 `save_model_template`。来源提供商与模型 ID 不作为路由绑定，拉取本身不改配置，也不需要运行时连接。
