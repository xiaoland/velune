# Agent 运行时

本 unit 实现版本化 Agent 运行时适配器：Pi RPC／SDK、Codex app-server 和 DeepSeek Harness ACP。它负责原生控制、会话读取与消息 projection；真实会话由各 Harness 持久化，Velune 不复制为另一套会话历史。Pi 适配器另提供显式来源的认证交互和只读提供商发现。

它只依赖 [会话契约](../conversation/README.md)，不依赖 AI 服务、网关、application 或 UniFFI。执行配置与网关注入的临时 token 由 application 显式传入；它不选择上游模型路由、保存应用配置，也不拥有原生界面。

`authentication` 与 `provider_source` 通过本包资源中的固定 Pi SDK helper 执行来源操作，返回非秘密的类型化状态和发现结果。application 负责将结果导入网关及原子提交；application 的提供商私有认证解析器通过固定 `pi_auth.mjs` 委托 OAuth 来源解析与刷新，不读取环境里的默认认证。

执行侧通过 `pi_rpc.mjs` 装配固定 SDK。所选 CLI 必须对应 Pi 1.0.2 安装；原运行时目录继续拥有设置和会话，模型目录与 selection 文件由 application 在独立位置注入。执行侧 ModelRuntime 使用空认证存储及临时网关 token，不读取原 auth 或继承上游 provider 环境密钥。提供商导入仍只读原来源，并排除 Velune 自己生成的网关提供商。来源读取失败通过 `SourceReadError` 保留白名单错误类别、阶段、退出状态和固定中文说明；SDK stderr、错误消息及配置内容不进入日志或界面。unit 只发出 tracing 事件，由消费方装配日志订阅器，RPC 读取线程显式继承其 dispatcher 与 span。

Pi 来源适配器按固定 SDK 1.0.2 的协议与有效兼容要求判断可导入性。Chat Completions 的已知设置只在当前网关请求能保持其语义时放行；原提供商和 URL 推断的输出字段、推理格式等默认要求也参与判断。`max_tokens` 来源设置通过可选 Pi 投影交给 application 转换为提供商模型的类型化 wire 选项，adapter 不依赖 AI-provider。未知字段或需要未实现 wire 行为的设置逐项说明原因，不将所有非空 compat 对象一概拒绝，也不把旧 Codex 后端或 Messages API 冒充受支持的 OpenAI 协议。

Pi 固定版本及协议边界见 [开发说明](../../docs/development.md)。SDK 与 Node 资源只随需要本地执行的 app 装配，不进入禁用 local-runtime 的绑定产物。

## 版本与原生控制

运行时类型是 versioned variant，family 仅用于展示分组，不作为派发键。当前三个 variant 为 `pi-1.0.2`（family `pi`）、`codex-0.159.3`（family `codex`）、`dsh-acp-0.2.0-rc.2`（family `deepseek-harness`）。各自版本 regex 是 `^1\.0\.2$`、`^0\.159\.3$`、`^0\.2\.0-rc\.2$`。启动前探测配置的 CLI 版本；协议握手继续检查所需能力。不同 breaking-change 版本应增加独立 variant 与 adapter，不能放宽 regex 假定兼容；多个 variant 与同 variant 的多个实例可并存。

Codex 使用 app-server 的 thread／turn 控制；DSH 使用 ACP v1 的 session/new、resume、prompt、cancel、set_config_option。DSH 的 agentInfo.version 是 ACP 插件版本，不用于判断 CLI 版本。huihua `0.2.0` 的公开 provider API 读取 Codex／DSH 历史，明确接收配置的 home 与 roots，内部按原生 ID 定位，未知记录不原样输出。它只提供历史 snapshot，不能替代原生 resume；Pi 保留 SDK SessionManager 的分支与模型选择语义。

原生权限或回答请求映射为 conversation 的类型化交互，用户显式选择、回答或取消；adapter 不自动批准。执行模型由当前会话选择后注入，运行时配置不含默认模型；Codex／DSH 历史没有 Velune 提供商身份时须明确选择，不能凭 API model ID 猜测。DSH 的模型切换关闭会话、更新网关注入后重启并 resume 同一原生 ID。DSH reasoningEfforts 需要实际 wire 映射，当前仅有能力等级列表，不猜测映射，保留 SDK 默认行为。

DSH 执行 overlay 禁用 settings、llm-deepseek 与 llm-deepseek-account，并为 llm-pi-ai／ACP 配置网关目录，防止原来源设置覆盖执行路由；不删除原来源配置。CLI 会按上游行为准备其 ACP profile。实例的 CODEX_HOME／DSH_HOME 与会话 cwd 分开；网关注入文件属于 application 的运行时投影目录。

Node 与用户配置的 Codex／DSH 可执行文件是外部依赖。huihua 固定依赖、完整许可证和安装入口见 [runtime-support 声明](runtime-support/THIRD_PARTY_NOTICES.md) 与 [开发说明](../../docs/development.md#原生运行时与历史读取)。仅 Pi SDK 与 huihua 依赖随 Mac 打包，不分发整个 DSH runtime 或 Codex CLI。

版本注册表同时声明中立 `RuntimeProtocol` 支持集合。application 将其转换成网关协议描述供界面使用，执行准备也检查同一集合；family 不参与协议资格判断。
