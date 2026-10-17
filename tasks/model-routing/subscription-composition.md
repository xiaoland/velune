# 订阅接入的组合边界

研究日期：2026-10-17。目标是保持一个订阅一个提供商实例，将可复用机制与具体服务接入分离；下一步对象为 Claude Code 订阅。此文件保存证据与候选方案，不代表已授权 Claude CLI 桥接或已经完成产品重构。

## 核心证据

Magpie 本次固定提交为 `dccfefdf722ea25f5cf2be1798d5686e43b89fbd`。其 [Claude subscription bridge](https://github.com/yetone/magpie/blob/dccfefdf722ea25f5cf2be1798d5686e43b89fbd/internal/gateway/claude_subscription.go#L3-L27) 明确运行 Claude Code 本体，不以 OAuth token 重放 HTTP；调用方工具通过 MCP 交给 Claude，工具调用返回外层调用方，下一次请求的工具结果恢复同一 Claude turn。进程和会话跨轮次保留，因此它不是 Messages HTTP 的认证头适配。[额度实现](https://github.com/yetone/magpie/blob/dccfefdf722ea25f5cf2be1798d5686e43b89fbd/internal/provider/claude_usage_cli.go) 通过 Claude CLI 取得用量。源码中的订阅资格／Extra Usage 理由是 Magpie 作者对适配必要性的解释，本轮未对真实账户验证。

官方资料有需要区分的适用范围：[帮助中心](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) 的 2026-10-07 更新说 SDK、claude -p 和第三方 app 可使用订阅额度；[Claude Code 认证使用条款](https://code.claude.com/docs/en/legal-and-compliance#authentication-and-credential-use) 同时明确限制第三方自有登录、收集／保存／中介 Claude.ai 凭据及代用户通过 Free／Pro／Max token 路由。不能据前者推导第三方 raw HTTP 订阅代理获准，也不能把 Magpie 的实现当供应商认可。未改动的 Claude Code 由终端用户自行登录是文档另行描述的路径。

## Velune 当前耦合

| 位置 | 当前行为 | 解耦意义 |
| --- | --- | --- |
| application/provider_authentication.rs | CodexSubscription 认证变体 | 认证适配有服务专属实现是合理的，不应控制所有其他能力 |
| application/subscription_quota.rs | 从 Codex 认证变体推导额度来源 | 额度来源应独立装配，仍与同一个账户身份／generation绑定 |
| gateway/runtime.rs | 只对 Codex profile 执行401刷新恢复 | 网关应消费认证恢复能力与稳定身份，而非品牌判断 |
| ai-provider/responses.rs | Codex目标、请求头、字段、MIME专例；subscription bool约束stream/store | 专属后端请求约定不能变成所有订阅的共同规则 |
| ai-provider/messages.rs | subscription bool直接拒绝Messages | 订阅是商业／认证关系，不是“不支持Messages”的协议性质 |
| gateway/scheduling.rs | QuotaSource与窗口中立，路由策略中立 | 保留现有共同机制，不为新服务复制调度器 |

## 候选组合

提供商实例组合 LLM 协议、执行后端、认证来源、请求适配与额度来源。HTTP执行继续采用现有原生协议；必要时CLI执行是独立能力，不能伪装为普通HTTP协议。认证适配负责登录、刷新所有权、身份与目标绑定；请求适配拥有额外headers／字段约束／流约定；额度来源只发布可验证窗口和可用性，不将缺失值填零。模型目录和实际能力仍由提供商模型拥有。

品牌预设只是已知可用的组合，不增加贯穿路由的品牌provider类型；也不允许用户任意拼接一个OAuth令牌与不匹配端点。编译期具名能力与装配校验先满足Codex现有路径，不建立动态插件平台、任意脚本或表达式DSL。实际不同的服务协议仍需维护适配器，共用机制不能消除上游差异。

Claude CLI若被接受，聚合、额度准入、冷却及候选选择仍在Velune gateway；Claude Code只负责一个成员的执行。新增代价包括运行依赖、进程与会话生命周期、工具结果往返、取消与错误映射，不能以“再加一个额度协议”概括。是否接纳该后端属于本轮需明确的产品范围，而非已经确认的实现细节。


独立设计判断：先把 Codex 接入 profile 从短期凭据中分出，gateway 通过认证恢复能力处理401，application 装配认证与额度来源，保留同一身份版本与目标绑定。CLI后端若加入，必须另外定义尝试是否已产生副作用；“尚未给下游发送HTTP headers”不足以判断CLI操作能否安全重试。可以暴露 Messages 兼容入口，但不能将其称为 Messages HTTP 上游。


工具执行边界的源码核对：Magpie 的 [CLI 参数构造](https://github.com/yetone/magpie/blob/dccfefdf722ea25f5cf2be1798d5686e43b89fbd/internal/gateway/claude_subscription.go#L1900-L1916) 关闭普通原生工具、使用指定MCP，并按请求开放 WebSearch；普通调用方工具挂起等待外部结果，不是任意执行 Bash。仍有可执行搜索及CLI自身认证／会话状态变化，因此已有HTTP提交闸门不能证明重试不会重复执行。Velune若接受桥接，应显式约束工具执行所有权并建立后端提交／取消契约，而非照搬Magpie的权限参数。
