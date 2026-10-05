# Agent 运行时

本 unit 实现 Pi 子进程 RPC、会话列表 SDK 接入、原生事件 projection、模型投影元数据，以及显式来源的认证交互和只读提供商发现。真实会话及认证存储仍由 Pi SDK 持有；Velune 不复制凭据或新建会话历史。

它只依赖 [会话契约](../conversation/README.md)，不依赖 AI 服务、网关、application 或 UniFFI。执行配置与网关注入的临时 token 由 application 显式传入；它不选择上游模型路由、保存应用配置，也不拥有原生界面。

`authentication` 与 `provider_source` 通过本包资源中的固定 Pi SDK helper 执行来源操作，返回非秘密的类型化状态和发现结果。application 负责将结果导入网关及原子提交；平台秘密 helper 通过 sealed `credential_source.mjs` 委托原来源解析，不读取环境里的默认认证。

执行侧通过 `pi_rpc.mjs` 装配固定 SDK。所选 CLI 必须对应 Pi 1.0.2 安装；原运行时目录继续拥有设置和会话，模型目录与 selection 文件由 application 在独立位置注入。执行侧 ModelRuntime 使用空认证存储及临时网关 token，不读取原 auth 或继承上游 provider 环境密钥。提供商导入仍只读原来源，并排除 Velune 自己生成的网关提供商。来源读取失败通过 `SourceReadError` 保留白名单错误类别、阶段、退出状态和固定中文说明；SDK stderr、错误消息及配置内容不进入日志或界面。unit 只发出 tracing 事件，由消费方装配日志订阅器，RPC 读取线程显式继承其 dispatcher 与 span。

Pi 固定版本及协议边界见 [开发说明](../../docs/development.md)。SDK 与 Node 资源只随需要本地执行的 app 装配，不进入禁用 local-runtime 的绑定产物。
