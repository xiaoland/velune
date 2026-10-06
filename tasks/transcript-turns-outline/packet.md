# 消息工作过程与 Outline

状态：实施中，2026-10-06。

用户授权改进消息列表：投影 turn，折叠过程消息并显示“工作了 <duration>”；提供用户消息 outline sheet；底部与 outline 入口同区，底部仅箭头；设置提供仅用户消息的 outline 模式，点击从其位置展开后续列表。用户建议 canonical 会话／消息与 forward-only 投影归 agent-runtime，包边界正在核对。

本次同时修复本地问题详情丢失。已确认 HistoryError.safe_message 将原因泛化，而非窗口截断。产品诊断完整性已归 PRD；开发验证仍不得读取真实凭据、配置或会话。

责任：Mac 展示由既有 session_management owner 负责；错误原因链由 ai_service_audit owner 负责；主执行者协调契约、文档、静态检查、安装。关键包边界向 conversation_presentation_advisor 请求判断。guides/delegation.md 当前不存在，不按缺失文档猜测要求。

待核对：用户消息和最终无工具调用回复是否都保持可见；历史／流式终止边界与时间缺失行为；forward-only 不能意味着消息正文不可修订，也不能隐含持久化第二份会话。

验证：不新增自动化测试。类型检查、静态检查、构建与合成隔离手动脚本；产品 UI 由用户验收。

当前建议实施：用户和当前最后一条消息可见，中间内容折叠；轻量类型仍在 conversation，forward-only reducer 在 agent-runtime。已向用户给出两个选项以复核，暂按推荐方案推进，不能记为用户明确选择。真实 runtime 完成状态用于停止计时，不用“当前没有 Tool block”猜测流式终态。application 装配完跨 Harness 逻辑内容后统一投影；仅保留消息 ID／role，不复制正文，工作时长是本机观察区间，纯历史无值。

新增偏好 TranscriptPresentation：conversation／userOutline，经类型绑定保存到既有 schema 7 配置。新增字段使用默认值，不是旧契约迁移；用户配置不会因增加视图偏好被重置。

真实 Pi 工具流发现并修复两个既有 adapter 缺陷：空原生文本消息会生成系统行并绕过尾行去重；移除乐观用户副本，消费原生 user echo。Pi 1.0.2 公开包 `dist/modes/json-event.js`／`.d.ts` 明确将 `message_update` 转为仅含 usage 与 assistantMessageEvent 的增量帧，不再携带累计 message／partial；旧投影因此只在 message_end 显示正文。本次按该固定 wire contract 累积 text／thinking／tool 元数据，message_end 保留最终权威，真实 prefix 门验收不能自动超时放行伪装流式成功。

真实流式收尾暴露第三个问题：get_messages 的数组位置不能作为原生身份，空系统帧会改变序号，导致工作时长、outline 与缓存失效。SDK `SessionManager.buildSessionProjection()` 有 provenance，已选用 sourceEntry.id／条目内序号。运行时扩展通过原生消息对象引用关联 live 临时身份与最终原生身份，快照传递具名、幂等确认；不以正文／时间戳猜测、不新增持久映射。根执行者维护 canonical／UniFFI 契约；原生 identity 修复由原 advisor 转为实施 owner，Mac owner 处理展示状态确认。

2026-10-06 验证：全 workspace fmt/check/clippy（all-targets、all-features、-D warnings）及 bindings no-default-features check/clippy 通过。Mac strict Swift 编译通过；合成诊断脚本确认 provider import、Pi history read/list/parser 的原始 cause 经实际 UniFFI API 到达 UI，尾随 system 不隐藏最终回答。

真实公开 Pi 1.0.2 的隔离 loopback 验收通过：实际 Store → Transport → UniFFI → Pi → 原生 ChatCompletions → 工具 read → 后续回复与第二轮，共三次本地请求。人工脚本用明确流式门等待消息前缀，超时失败，不自动放行；确认同一 row 对象与 Markdown 缓存复用、工作时长保留、outline／Disclosure 原生身份确认后不丢失、晚到 user 只响应显式发送意图、偏好持久及关闭重开原生 ID 一致。所有配置、会话、文件、服务均为合成隔离内容，未读取真实凭据或调用真实模型 API。安装尚待最终干净构建；UI 产品体验留给用户。

认证公共失败边界的补齐也属于本次原因链修复：保留 JSON／I/O／进程状态原因，helper stderr 有界缓存并持续排空，仅在失败时附带。最终统一检查必须在各源码 owner 冻结后重跑，不能采用编辑中的中间构建结果。
