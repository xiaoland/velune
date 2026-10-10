# ACP 与 acpsub 参考证据

检查日期：2026-10-10。以下为公开源码与协议检查，不是运行时或真实模型验收。

## 来源与适用边界

用户更正的参考为 [lexoliu/acpsub](https://github.com/lexoliu/acpsub/tree/259db0d08f43941d3742d3a4710fd89f0adc603b)，版本 0.2.0，MIT。此前 `lexoliu/acphub` 不可读的记录描述的是拼写错误的地址，不继续用作本轮阻塞项。

官方协议基线为 [agent-client-protocol](https://github.com/agentclientprotocol/agent-client-protocol/tree/af9d8f5d3852f8c3c774b81f7366e6707e0fd5e1)，以 `schema/v1/schema.json` 与 `schema/v1/meta.json` 的稳定定义为准。ACP v1 的模型配置使用 `session/set_config_option`；被移除的实验性 `session/set_model` 不作为新适配器契约。

## 可采用的判断

`acpsub/src/state.rs` 的 `open_session` 在 Agent 声明 `loadSession` 时才调用 `session/load`，否则明确报不支持。`handshake` 先检查 Agent 公布的选项，再设置模型与模式，并读取回复中的有效值。对 Velune 的启示是保留“未请求模型覆盖”和“显式选择模型”的差别，不为无模型选择的 Agent 发明模型标识，也不把请求值当作 Agent 已接受值。

官方 `SessionConfigOption.category` 是可选语义分类；option ID 与 value 都归 Agent。模型选项应按已声明的分类识别，不能把所有 Agent 的 ID 固定为 `model`。select 可以是平铺或分组列表；更新通知中的完整选项列表替换原状态。当前稳定协议也包含 boolean 配置类型。

`sessionCapabilities.list/resume/close/delete` 使用可选对象声明支持，空对象表示支持，缺失或 null 表示不支持。`loadSession` 独立声明历史回放能力。`session/list` 有游标分页；`session/load` 的历史经 `session/update` 回放，不能把一次空响应当作无消息历史，也不能因不支持 load 而新建替代会话。

`acpsub/src/handler.rs` 提供可配置 permission policy、文件与终端回调；它的默认 allow 策略不构成 Velune 的授权。Velune 沿用调用方显式回答权限请求的契约，仅声明实际实现的客户端能力。未知方法返回协议错误，不能自动批准或假装执行客户端工具。

## 不随参考引入的职责

acpsub 的 MCP 服务、独立 daemon、registry、transcript 副本、排队与 steering 属于其产品。Velune 当前只实现通用 ACP 接入，继续由 Harness 拥有真实会话，不因参考而新增常驻 Host、第二套会话正文存储或 MCP 协作服务。

ACP 是客户端与 Agent 的控制协议，没有通用的 LLM 提供商端点／认证注入契约。现有 Pi、Codex、DSH、Claude 专用适配器继续承担各自网关接管；通用 Adapter 不猜测环境变量，也不把模型 option 的 opaque value 当作 Velune Gateway route。

## 覆盖更多 Harness 的判断

用户进一步说明目标后，核对 [官方 Agent 目录](https://agentclientprotocol.com/get-started/agents)：既有原生 ACP Harness，也有 Claude、Codex、Pi 等 bridge。通用 command＋argv＋显式 environment 足以表达这些启动入口，不能要求它们都安装 Node 或使用某种 agentDir。目录列名只证明存在公开接入，不代表 Velune 已逐一验收；不引入自动安装器或新的支持白名单。

采用已发布 `agent-client-protocol-schema =1.11.0`（禁用默认 features）的 v1 类型作为 wire 边界，保留既有进程／SSH 与同步轮询。schema 不承担会话 projection；完整异步 SDK 的连接生命周期不在当前改造范围。该选择旨在把可选字段和未知枚举的兼容规则交还协议权威，避免为了每个 Agent 反复修手写 JSON 假设。

发布版核对：Cargo 下载与 sparse index 确认 schema 1.11.0 于 2026-10-09 发布、未撤回，MSRV 为 1.88；不以缓存网页的 latest 字样决定版本。该版 `NewSessionResponse`／`LoadSessionResponse` 对可选 configOptions 使用跳过无效项的反序列化规则，未知 option 类型不应阻断 Agent 默认运行。
