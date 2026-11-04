# RelayKit 升级与桥接修复

## 目标与授权

用户先要求对照 QuantumNous/new-api 的 RelayKit 审查三协议转换，随后授权更新 RelayKit 并修正本地桥接。只修改相关依赖、网关、桥接、回归检查及说明；不提交、不重装应用。验证使用合成数据和本机回环上游，不读取真实凭据或会话。

## 当前结果

本次实现与合成验证已完成，六项初始缺陷均已复验消除；修改未提交。

- RelayKit 从 `v0.3.1-0.20261010122904-0f38a14a5e8e` 升至 `v0.3.6-0.20261102115434-45746eee4de2`，固定到已审查的 [main 提交](https://github.com/QuantumNous/new-api/tree/45746eee4de25ab1eb75d4b7fbb8de2199483ea3/relaykit)。对应模块校验值已写入 go.sum。Go 工具链为 1.26.2 darwin/arm64。
- [网关派发](../../packages/gateway/src/runtime/attempts.rs)改为每个实际候选分别准备翻译：先替换真实型号，再采用该模型的输出上限；排序选中其他成员或 fail-over 均重新准备，用该计划处理相应响应。未交付 headers 的失败尝试不产生转换响应。
- [Go 桥接](../../packages/ai-provider/relaykit-bridge/main.go)以 `json.Number` 保留原始数字，仍拒绝尾随 JSON；`n>1` 的 best-effort 降级相应适配。
- Messages SSE 的 usage 保留前面已经报告、当前片段省略的字段，再交给 RelayKit，修正上游仍存在的累计用量输出问题；显式零值保留，分拆缓存字段也按存在性合并。原始上游计量仍在 Rust 中独立观察。
- 新传递依赖为 MoeForge/moego；[许可清单](../../app/mac/Licenses/RelayKit/NOTICE.md)及原始许可同步更新，移除已不再链接的 tidwall 依赖许可。RelayKit 的原始 AGPL 全文与当前模块比较一致。
- [网关回归](../../packages/gateway/tests/dispatch.rs)新增实际请求检查，覆盖轮询和 fail-over 时不同 Claude 模型的推理规则及不同默认上限。

## 初始六项缺陷与复现依据

| 初始问题 | 原来可观察的错误 | 修复归属 |
| --- | --- | --- |
| P1：转换读到入口别名 | `velune/model/fixture` 让 Opus 4.7 使用 manual thinking；真实型号应产生 adaptive + effort | 本地候选派发时机 |
| P1：大整数失真 | schema enum 与工具输入中的 `9007199254740993` 变成 `9007199254740992` | 当前上游已保真，本地边界继续保真 |
| P1：连续 assistant 丢工具调用 | Chat 转 Messages 后 call_a 丢失，call_b 及两份结果仍存在 | 升级吸收上游修复 |
| P1：混合思考和正文的 chunk 丢正文 | Chat chunk 同时带 thinking/text，仅输出 thinking，仍正常结束 | 升级吸收上游修复 |
| P2：assistant 文本与工具同块时丢文本 | Messages 转 Chat 后 content 为 null | 升级吸收上游修复 |
| P2：流式最终用量丢输入 | Messages 的输入 10、输出 2 变成 OpenAI 最终输入 0、总计 2；上游累计 state 自身正确 | 本地补齐省略的已报告 usage 字段 |

三项上游修复位置：[连续工具消息](https://github.com/QuantumNous/new-api/blob/45746eee4de25ab1eb75d4b7fbb8de2199483ea3/relaykit/relayconvert/internal/oai_chat/to_claude_messages_req.go#L154)、[混合思考／正文](https://github.com/QuantumNous/new-api/blob/45746eee4de25ab1eb75d4b7fbb8de2199483ea3/relaykit/relayconvert/internal/oai_chat/to_claude_messages_resp.go#L342)、[工具与文本同块](https://github.com/QuantumNous/new-api/blob/45746eee4de25ab1eb75d4b7fbb8de2199483ea3/relaykit/relayconvert/internal/claude_messages/to_oai_chat_req.go#L273)。[当前上游流转换入口](https://github.com/QuantumNous/new-api/blob/45746eee4de25ab1eb75d4b7fbb8de2199483ea3/relaykit/relayconvert/response_registry.go#L1169)仍会用单片段 usage 覆盖累计输出，直接调用已证实该问题。

## 验证证据与当前步骤

临时探针和输出在 `/tmp/velune-relaykit-probe-20261102`，上游 checkout 在 `/tmp/velune-new-api-review-20261102`；不属于发行源码。`pinned` 为原始依赖，`current` 为原桥接替换当前上游，`fixed` 为本次修复后的实际桥接。通过 c-shared + ctypes 调用正式 C 入口，Go 二进制模块清单已确认是固定的新版本。

已通过：

- `probe.py fixed`、`extra.py fixed`、`controls.py fixed`：大整数、连续工具历史、混合 thinking/text、assistant 文本、两条 OpenAI 路径的最终 usage 10／2／12；六个异协议方向的 JSON/SSE 文本、截断拒绝与显式错误拒绝；Responses custom tool 还原。
- `edges.py fixed`：最大 u64 schema、n>1 降级、尾随 JSON 拒绝、缓存读写用量跨事件保留、后续显式零值。缓存样例最终输入 17／输出 2／总计 19。
- Go vet 与模块完整性校验。

Rust 类型检查已通过。`dispatch.rs` 的 7 项回归通过，覆盖同协议透传、错误、fail-over 与新增模型规则／上限检查；临时探针的 4 项新增检查通过：六向实际网关 JSON/SSE、下游断开后上游取消、缺上限候选让位、失败尝试仅交付最终错误。探针重用了现有 7 项测试，一并运行也通过。临时文件已移出仓库，源码保留为上述目录的 `gateway-probe.rs`。运行验证使用该目录内独立的 `cargo-target`，避免与其他构建争用锁。相关两包 Rust fmt、Go fmt、git diff --check、文档相对链接／标题锚点和许可文件存在性均已通过。`cargo clippy --locked -p velune-ai-provider -p velune-gateway --all-targets -- -D warnings` 已通过。全工作区 Rust fmt 本次运行曾观察到并行改动 `packages/agent-runtime/src/native/codex_title.rs:118` 的格式差异，本任务未修改该文件。

尚未验证真实供应商、真实 Harness、其他平台的打包与运行；本次不扩大这些能力声明。同协议继续绕过翻译。
