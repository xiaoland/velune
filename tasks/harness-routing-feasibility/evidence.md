# Harness 与协作：一手证据快照

检查日期：2026-10-01 UTC。以下是公开文档／仓库源码核对，**不是运行验证**。未读取真实凭据、安装／启动 Harness、调用模型、注册 OAuth 应用或部署。设计取舍见[架构方案](../../docs/design/architecture.md)；实验见 [Packet](packet.md)。

## 版本边界

| 对象 | 实际检查基线 | 适用边界 |
| --- | --- | --- |
| Codex | [rust-v0.159.3](https://github.com/openai/codex/releases/tag/rust-v0.159.3)，commit [01fc69f4026735edfdf6789820549727a4867b11](https://github.com/openai/codex/commit/01fc69f4026735edfdf6789820549727a4867b11) | 协议源码按 release；官方网站按检查日，不能反推该 binary 已具备网站全部能力 |
| Claude Agent SDK | Python [v0.2.163](https://github.com/anthropics/claude-agent-sdk-python/releases/tag/v0.2.163)，commit [1ef6d8c71bb0e44a6b33fe61497864f21e17fdb7](https://github.com/anthropics/claude-agent-sdk-python/commit/1ef6d8c71bb0e44a6b33fe61497864f21e17fdb7)，发布附带 CLI 2.1.286 | 检查了 Python SDK 生命周期源码；未检查闭源 CLI。推荐用 TypeScript 不代表已验证 TS 同版本行为，实施前另锁版本 |
| Pi | release [v0.99.2](https://github.com/earendil-works/pi/releases/tag/v0.99.2)，commit [005af57d88ee23b33778f343a9595b32e67ff788](https://github.com/earendil-works/pi/commit/005af57d88ee23b33778f343a9595b32e67ff788)；路由细查 HEAD [e792ba131ed0495f3ff58a0eb13f20540e344d5c](https://github.com/earendil-works/pi/commit/e792ba131ed0495f3ff58a0eb13f20540e344d5c)，提交时间 09:37:01 UTC | 两个 SHA 不混为一版；HEAD package 仍标 0.99.2。实施锁定具体 commit，并比对 release 的对应路由能力；原 badlogic/pi-mono 转向 earendil-works/pi |
| ACP | 已发布 v1 文档，检查日快照；[v2 公告](https://agentclientprotocol.com/announcements/acp-v2-draft)仍为 draft | 按每个 adapter 协商的能力使用；不预设支持所有可选方法 |

网站是可变资料，后续实现必须重新核对；本文件不为可变网页伪造 commit。

## Codex

### C1 原生控制与会话

已验证：app-server 有 thread start／resume／fork／list／read 和 turn start／steer／interrupt。`turn/steer` 不能改变模型；轮次启动可指定 model／effort。resume 能重接本 app-server 已知的运行线程或加载存档，不是任意终端进程接管。JSON-RPC acceptance 早于完成，审批保留 thread／turn／item 与原生 request 标识。

- [发布版方法](https://github.com/openai/codex/blob/01fc69f4026735edfdf6789820549727a4867b11/codex-rs/app-server-protocol/src/protocol/common.rs)
- [发布版 thread 恢复](https://github.com/openai/codex/blob/01fc69f4026735edfdf6789820549727a4867b11/codex-rs/app-server-protocol/src/protocol/v2/thread.rs)
- [官方 app-server](https://learn.chatgpt.com/docs/app-server)：原生账户用量可读；部分 shell/process 控制接口在 thread sandbox 外，不能当普通 sandboxed tool 暴露；非本地监听必须单独保护

### C2 请求路由与认证

已验证：自定义 provider 的 base URL、凭据引用、headers 与 retry 参数可配置；当前 wire API 是 Responses，不能拿 Chat Completions 兼容直接替代。ChatGPT 原生登录与 API key 计费是不同路径；`requires_openai_auth` 的行为也不同于普通 env key。`CODEX_HOME` 可隔离文件状态，仍须验证 keyring／进程边界。

- [配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)
- [认证](https://learn.chatgpt.com/docs/auth)

### C3 正式 Sign in with ChatGPT（SIWC）计划用量路径

这比“只能在原生工具用订阅”的旧假设更开放，但有明确边界：

- [资格入口](https://developers.openai.com/siwc/token-sharing-open-source)：开源／本地应用有正式路径；付费或远程托管应用转合作申请。app-server 旧认证不能作为商业／托管许可
- [注册](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)：PKCE／state／nonce、真实应用归属、host 标识和 issued client ID；不能把 bootstrap client 当长期注册
- [账户与会话](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions)：registration 按实际用户和 workspace 隔离，即使邮箱相同；串行 refresh，禁止拼装不同 registration 的身份与 token
- [模型与推理](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)：查询当前账户模型；公开 `/v1/responses`，不用 `backend-api`；app-server 缓存模型列表不能证明 entitlement
- [app-server 集成](https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server)：正式环境变量 provider 路径在 token 更新后需重启／恢复进程，不能假定运行中热更新
- [preview 限制](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)：HTTP `store:false`、`stream:true`、完整所需历史、不用 `previous_response_id`；并非一般 Responses API 的所有参数／托管工具都能用
- [错误恢复](https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery)：app-specific sharing limit 不证明整个计划耗尽；不能悄悄切成 API 付费，不对永久资格／政策错误循环 OAuth

**设计推论**：Codex 与其他符合资格的 Harness 可经正式 SIWC 接口利用 ChatGPT 权益，但必须单独确认应用形态、注册与能力。不是所有已有 token 都能拿来转发，也不是本产品已经获得托管许可。

## Claude Code

### A1 原生订阅与网关

已验证：`ANTHROPIC_BASE_URL` 接网关；若存在 gateway credential／apiKeyHelper，使用该凭据替代订阅，产生对应 API／云供应商费用；仅设置 BASE_URL 时保留原生 OAuth。官方明确不支持网关下的非 Claude 模型。

- [网关总览](https://code.claude.com/docs/en/llm-gateway)
- [连接与凭据](https://code.claude.com/docs/en/llm-gateway-connect)
- [model 配置](https://code.claude.com/docs/en/model-config)：模型／alias／SDK setModel／subagent 配置存在多级优先级，物理路由不能与能力假设失配

### A2 产品托管、认证与隔离

已验证：官方允许符合条件的平台运行未修改的 Claude Code binary，用户自行使用自己的订阅／API／云凭据登录；保留内建登录方法。另明确禁止第三方收集、储存、中介 Claude.ai token 或自行提供 Claude.ai 登录。因此技术上能 OAuth pass-through，不等于可建立产品订阅 token pool。

`CLAUDE_CONFIG_DIR` 支持隔离设置、会话、凭据及目录绑定的 macOS Keychain；无 API key 的 Console sign-in 有存储例外，不能把 config dir 当万能隔离证明。

- [法律与合规](https://code.claude.com/docs/en/legal-and-compliance)
- [认证与多配置目录](https://code.claude.com/docs/en/authentication)

### A3 网关兼容契约

已验证：Messages 流事件和 pings、协议 headers、功能 body、错误语义必须保持；beta／body 配对、推理签名、cache 结构均可能影响兼容。部分非推理出站绕开 base URL。存在会话／agent 属性及可选路由提示；hint headers 至少 v2.1.273，prompt ID 至少 v2.1.283，managed destination pinning 至少 v2.1.285。

- [官方协议指南](https://code.claude.com/docs/en/llm-gateway-protocol)

**设计推论**：先保真支持 Claude 多上游；跨模型家族翻译须独立实验，不能用“协议兼容”跳过工具、签名、上下文与升级回归。

### A4 生命周期与权限

已验证：用显式 session ID resume／fork；continue 选最近会话，不适合产品并发会话。fork 历史不等于复制文件。SDK 的私有 control_request envelope 不等于通用 JSON-RPC；让 SDK 管理它。Python 发布版源码显示 result 只结束一轮，后台工作仍可能继续。

- [会话文档](https://code.claude.com/docs/en/agent-sdk/sessions)
- [固定版本 query.py](https://github.com/anthropics/claude-agent-sdk-python/blob/1ef6d8c71bb0e44a6b33fe61497864f21e17fdb7/src/claude_agent_sdk/_internal/query.py)
- [权限处理](https://code.claude.com/docs/en/agent-sdk/permissions)与[交互／审批](https://code.claude.com/docs/en/agent-sdk/user-input)：自动获准工具可能绕过 canUseTool；必须覆盖所有工具时用 PreToolUse，保留一次许可与持久 permission 修改的区别
- [Remote Control](https://code.claude.com/docs/en/remote-control)是 Claude 自身界面能力，不证明本产品可接其第三方控制 API

## Pi

### P1 原生逐请求路由

HEAD 已有 `registerVirtualModel().route()`，在 user／continuation／retry／direct 请求前选择物理模型，可用 previous／failed model、transcript、abort、分支持久 router state；路由选择和实际发出请求有各自记录。文档建议工具延续／重试保持粘性以保护 cache 与推理签名。

- [固定 HEAD virtual models](https://github.com/earendil-works/pi/blob/e792ba131ed0495f3ff58a0eb13f20540e344d5c/packages/coding-agent/docs/virtual-models.md)
- [provider 扩展](https://github.com/earendil-works/pi/blob/e792ba131ed0495f3ff58a0eb13f20540e344d5c/packages/coding-agent/docs/custom-provider.md)：优先复用原生 stream／tool 转换，custom stream 必须平衡事件、正确终止与取消
- [模型与凭据优先级](https://github.com/earendil-works/pi/blob/e792ba131ed0495f3ff58a0eb13f20540e344d5c/packages/coding-agent/docs/models.md)
- [独立认证存储示例](https://github.com/earendil-works/pi/blob/e792ba131ed0495f3ff58a0eb13f20540e344d5c/packages/coding-agent/examples/sdk/09-api-keys-and-oauth.ts)
- [OpenAI provider](https://github.com/earendil-works/pi/blob/e792ba131ed0495f3ff58a0eb13f20540e344d5c/packages/ai/src/providers/openai.ts)：当前有正式 SIWC public Responses 路径，旧 openai-codex 单列；OAuth 实现存在不等于任何供应商都授权该用途

### P2 原生会话与终止语义

已验证：RPC 为按行 JSON，不是 JSON-RPC 2.0；response 可乱序，多数事件无命令 ID。prompt acknowledgement 是接受／排队，不是完成；等待 `agent_settled`，不能只等 `agent_end`。停止全部工作先 `clear_queue` 再 `abort`，否则排队内容可能继续。原生历史 entry tree 不是产品任务树。

- [v0.99.2 RPC](https://github.com/earendil-works/pi/blob/005af57d88ee23b33778f343a9595b32e67ff788/packages/coding-agent/docs/rpc.md)
- [命令契约](https://github.com/earendil-works/pi/blob/005af57d88ee23b33778f343a9595b32e67ff788/packages/coding-agent/docs/rpc-commands.md)
- [RPC 实现](https://github.com/earendil-works/pi/blob/005af57d88ee23b33778f343a9595b32e67ff788/packages/coding-agent/src/modes/rpc/rpc-mode.ts)

### P3 MCP 与安全边界

已验证：当前原生 MCP 支持 stdio 与 streamable HTTP，非旧式 SSE；项目 MCP／扩展配置受 trust 控制。Pi 没有内建沙箱和逐工具审批；扩展、子进程共享 OS 权限。tool_call 扩展可阻断，不能替代 OS 隔离。RPC 的默认 ask 无法显示内建项目 trust 询问，未决定时跳过受保护项目资源。

- [MCP](https://github.com/earendil-works/pi/blob/005af57d88ee23b33778f343a9595b32e67ff788/packages/coding-agent/docs/mcp.md)
- [扩展接口](https://github.com/earendil-works/pi/blob/005af57d88ee23b33778f343a9595b32e67ff788/packages/coding-agent/docs/extensions.md)
- [安全说明](https://github.com/earendil-works/pi/blob/005af57d88ee23b33778f343a9595b32e67ff788/packages/coding-agent/docs/security.md)

## ACP、MCP 与 ACPHub

### I1 ACP 的真实范围

ACP v1 具有 prompt／update／permission／cancel，并有可选的会话 list（过滤／分页）、load（历史 replay）、resume（无 replay）、close。不能说 ACP 没有发现能力；它是 agent 已知会话的发现，不自动覆盖任意活动进程／整个集群。取消后仍须处理晚到事件，以原 prompt 完成响应确认；标准 stdio 与 draft HTTP 不混用。

- [v1 总览](https://agentclientprotocol.com/protocol/v1/overview)
- [session/list](https://agentclientprotocol.com/protocol/v1/session-list)
- [session lifecycle](https://agentclientprotocol.com/protocol/v1/session-setup)
- [prompt turn](https://agentclientprotocol.com/protocol/v1/prompt-turn)
- [transports](https://agentclientprotocol.com/protocol/v1/transports)

### I2 MCP 的位置

MCP 定义工具／资源／prompts；可承载产品 broker 工具，但并不定义本产品的任务树、跨 Harness 原生会话绑定、控制者租约和业务消息恢复。这个边界是设计判断，不是声称 MCP 缺少任何扩展能力。

- [官方架构（2026-07-28）](https://modelcontextprotocol.io/docs/2026-07-28/learn/architecture)

### I3 lexoliu/acphub 检查结果

精确参考：[lexoliu/acphub](https://github.com/lexoliu/acphub)。[owner 页面](https://github.com/lexoliu)可读；仓库 GitHub API 与精确 main README contents API 都返回 404，web repository／tree／raw main/master 读取失败，git 读取也未取得公开引用。

结论仅是**本轮授权／公开读取路径不可访问**，无法判断私有、删除、改名或拼写问题。未取得源码、版本、license 或架构，因此本轮没有“检查过 ACPHub 实现”的结论，也不拿同名项目替代。正确可读来源是唯一缺失的指定参考输入；主方案不依赖它。

未来对照检查：它是否能提供有许可的 transport／registry／adapter、是否忠实透传审批和取消、是否支持事件恢复与单控制权、能否在不接收订阅凭据的前提下接入。若只提供 ACP 转发，就放在边缘；即使提供更多能力，任务与消息契约也保持产品可迁移。

## 原生客户端与 Rust 边界

新增核对：2026-10-01 UTC。以下为官方平台文档事实；Rust 模块、Mac Catalyst 首选和移动端 Host 分工是设计推论，未编译或验证任何目标。

- **N1 Apple UI**：[Apple UI 框架概览](https://developer.apple.com/documentation/technologyoverviews/app-design-and-ui?language=objc)、[Xcode 多平台 target](https://developer.apple.com/documentation/Xcode/configuring-a-multiplatform-app-target)、[UIKit](https://developer.apple.com/documentation/uikit)明确区分 UIKit、Mac Catalyst 与 macOS AppKit。Catalyst 将 iPad/UIKit 应用适配到 Mac；“Designed for iPad”原样运行到 Apple silicon 是另一种目的地。推荐 iOS／iPadOS UIKit＋Mac Catalyst 的 Mac 客户端，不把它叫 AppKit 应用，也不由此承诺所有 Apple OS
- **N2 Apple 后台**：[策略选择](https://developer.apple.com/documentation/BackgroundTasks/choosing-background-strategies-for-your-app)、[持续长任务](https://developer.apple.com/documentation/BackgroundTasks/performing-long-running-tasks-on-ios-and-ipados)、[WWDC25](https://developer.apple.com/videos/play/wwdc2025/227/)：普通后台完成时间有限，部分任务启动由系统决定；iOS／iPadOS 26 的 BGContinuedProcessingTask 可让用户前台发起的工作延续到后台，但有取消、进度和资源限制。不能说移动端绝不可能长跑；也不能把该机制当永续 coding daemon
- **N3 Android**：[Compose](https://developer.android.com/compose)、[前台服务启动限制](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)、[WorkManager 长任务](https://developer.android.com/develop/background-work/background-tasks/persistent/how-to/long-running)：Compose 是原生 UI 工具；前台服务启动有条件，Android 16 的长时 worker 可耗尽 JobScheduler 配额。某些 service type 的专门 timeout 不应泛化为所有 Android 后台任务限制
- **N4 Rust FFI**：[UniFFI 官方指南](https://mozilla.github.io/uniffi-rs/latest/)支持生成 Swift／Kotlin 绑定，但不替代平台库分发／打包；[Rust FFI 指南](https://doc.rust-lang.org/nomicon/ffi.html)提供 C ABI／库与异步回调边界。UniFFI 只是推荐的首个验证对象，不是用户已选依赖；FFI 不等于进程间通信，也不保证 mobile 拥有 desktop 的进程权限
- **N5 SQLite**：[WAL 官方说明](https://www.sqlite.org/wal.html)要求同主机共享状态、同一时刻一个 writer，不能作为网络文件系统上的多设备共享库。设计推荐 Host 本地权威库＋客户端独立缓存，网络传领域命令／事件，不同步打开中的 DB／WAL 文件

上述平台事实不改变首要路线：先完成三个固定 Harness 的本地路由和协作；手机原生界面可以提前验证契约，但不是将工程重心改成手机远控。
