# 速度即模型变体

## 目标与授权

2026-11-04 用户决定推翻 messaging-and-workspace-polish 中"速度=按轮 service tier 选项"的产品方向，改为**速度=目录组合**：

1. managed 模型记录支持自定义额外请求参数（`request_parameters`），快速配置声明为一条独立模型记录（同 provider_model_id + `service_tier` 等参数 + 昵称）。
2. 运行时原生模型由 adapter 自己摊平：Codex adapter 把目录报告的 serviceTiers 展开为伪模型条目（如 `xxx · 快速`），选择后由 adapter 解析并在线上参数中传递。
3. 删除跨切面的 service_tier 维度：ModelServiceTier 契约、NextTurn.service_tiers、TurnChoice.service_tier、validate_service_tier 的 runtime-kind 闸门、各端速度菜单与持久化。

用户明确："删除代码不是代价；请执行。" 授权本范围必要源码修改与文档改写。不提交、不重装日常应用。

设计理由（用户+讨论共识）：速度是 (提供商, 模型, 账户) 的服务属性，不是运行时属性；谁拥有目录谁声明；核心与 UI 不与协议耦合，协议知识只存在于 adapter；managed 派发时 gateway 注入上游，runtime 无需感知，因此按 runtime kind 设闸在架构上错误。

## 关键事实

- 速度相关代码全部未提交（HEAD 无 service_tier）；涉及文件见盘点：agent-runtime（native/mod.rs、codex.rs、claude.rs、deepseek.rs、acp/mod.rs、client.rs、lib.rs、transcript.rs、codex_history.rs、codex_retry.rs）、application-api（command.rs、config.rs、next_turn.rs、records.rs）、application（commands.rs、composer.rs、local/* 多处）、bindings/lib.rs、conversation/lib.rs、Mac（Composer.swift、ComposerContext.swift、ConversationStore.swift、Transport.swift）、Windows（Conversation.FollowUp.cs、ConversationView.xaml.cs）、iOS（ConversationModel.swift）、Android（ConversationModel.kt）。
- `ai-provider/src/responses.rs` 的 `codex_request` 已有 service_tier 白名单（保留 priority/ultrafast），作为 Codex 订阅 profile 的净化步骤保留。
- ProviderModel 字段：record_key、provider_model_id、nickname、icon、context_window、max_output_tokens、reasoning_levels；ModelTemplate 对称。ModelRoute 成员是 record_key，快速变体自动可进路由。
- Codex 0.166 线上参数为 `turn/start.serviceTierForTurn`，0.159 为 `serviceTier`（未提交代码已实现，迁移到 adapter 内部）。Codex 官方 schema 的 `Model.serviceTiers` 已核实。
- Kimi 高速版走独立模型 ID（官方接口），与本方向一致。

## 方案要点

- 伪模型 id 编码需无歧义且 adapter 只解析自己铸造的 id。
- `request_parameters` 合并契约：adapter 建完请求体后合入，冲突用户值优先，之后仍过 profile 净化；上游不认识的参数以 400 如实报错。
- 推理强度维度不变。
- 速度菜单删除后，"当前速度"即"当前模型"，无需"不继承上一轮"特殊规则。

## 验收

- cargo check/test 覆盖受影响 crate；删除后无 service_tier 残留（除 ai-provider 净化与 adapter 内部线格式）。
- Codex adapter 伪模型：目录展开、id 解析、0.166/0.159 线参数的单元测试。
- request_parameters：合并语义（用户值优先、非对象拒绝）单元测试。
- Mac 构建通过；Windows/iOS/Android 改动端构建或至少绑定再生成后编译检查。

## 当前进展

实现已完成的部分：

- 删除跨切面 service_tier 维度：conversation（TurnSettings）、agent-runtime（TurnInput、ModelOption.service_tiers、ServiceTier、resume 参数、重试与历史记录）、application-api（ComposedTurn、TurnChoice、RuntimeModel.service_tiers、ModelServiceTier、ResumeTurn、next_turn 全部档位逻辑与测试）、application（Prompt、校验、发送、继续、队列、侧聊、计划任务、composer 测试）、bindings（resume_turn）、四端 UI（Mac 菜单与 ComposerContext、Windows 速度子菜单、iOS/Android 的 ComposedTurn/TurnChoice 构造）。
- Codex adapter：`model/list.serviceTiers` 摊平为变体模型（id `velune:codex-speed:` 加 JSON 二元组（base、tier），名称 `<显示名> · <档位名>`）；select_native_model 解析变体 id，档位存会话状态，`codex_turn_params` 在非 gateway 轮携带 serviceTierForTurn（0.166）／serviceTier（0.159）；新增 split_speed／speed_model_name 单元测试；`native::display_model_name` 供 transcript 命名。
- managed `request_parameters`：application-api ProviderModel（Option<StringMap>）→ application 内部记录（含 pi_binding_id 身份与双向转换）→ gateway config（校验：键非空无控制字符、仅 HTTP 后端）→ RouteTarget → attempts 合并（翻译后合入，用户值优先）；dispatch 测试断言参数到达上游请求体。
- UI 编辑：Mac ProviderModelDraft 加额外请求参数（逗号分隔 key=value，格式校验）；Windows ModelRow 同步。
- 文档：docs/prd/composer.md（速度=目录组合）、application README（resume_turn 签名与速度段）、agent-runtime README（Codex 档段落）、ai-service.md（模型条目能力）、PARITY.md、sources.md 新增 S77、旧任务包标记取代。

工作区收尾已通过 workspace/all-features/all-targets 严格 Clippy；速度组合改为 adapter 命名空间内的无歧义 JSON 二元组，普通含 `~` 的模型 ID 不被误解析。实际 Codex 0.166 原生目录变体→回环 Responses 请求验证通过：上游模型不变，变体携带 priority，选择普通模型清除档位。managed 自定义参数也通过实际 Claude 网关请求捕获确认。平台构建与提交授权由[工作区收尾任务](../workspace-delivery-and-network/packet.md)接管，不再沿用本包最初的“不提交”边界。
