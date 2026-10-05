# Pi DeepSeek 风格来源：导入功能验收

2026-10-05，针对已安装的 `0.1 beta.1` 验收。bundle manifest 的 `source_commit` 为 `7345ba8d18087982d5cd726daf797a9ea33a4397`，`dirty: false`。正常入口加载已安装 bundle 的 Rust 动态库和同一份 Resources，未以工作区库代替。固定 Pi SDK 为 1.0.2，人工脚本使用 Node 24.15.0。

**结论：正常导入验收失败。** 已保存的 Pi 运行时可以发现合成来源，但 DeepSeek 风格模型不可选择，调用正常 apply API 也被拒绝，无法由这一入口继续完成路由、连接及发送。先前验证标准 Chat Completions 或把不支持配置正确拒绝，不能作为这一来源的验收通过证据。

## 实际观察

所有来源文件、应用 HOME 和请求均为隔离的合成数据；模型端点仅为 loopback，没有读取用户模型、认证或会话，也未调用真实提供商。合成来源使用 `openai-completions`，显式设置 `thinkingFormat: "deepseek"`、`requiresReasoningContentOnAssistantMessages: true` 和 `maxTokensField: "max_tokens"`，分别提供 `reasoning: true` 和 `reasoning: false` 模型。

| 路径 | 观察 | 结论 |
| --- | --- | --- |
| 正常配置实例 → preview → apply | 运行时保存成功；推理模型 preview 禁选，apply 拒绝；理由是推理格式与 assistant reasoning 历史设置不支持 | 导入失败，后续正常链路不可达 |
| 同配置的非推理模型 | preview 同样禁选 | 当前判定没有充分考虑 `model.reasoning`，存在过度拒绝 |
| 固定 SDK 第一次实际 loopback 调用 | 发送 `thinking: {type: "enabled"}`、`reasoning_effort: "high"` 和 `max_tokens`；接收包含推理内容的工具调用 SSE | SDK 能产生并接收所需协议内容 |
| 固定 SDK 工具续轮 | 前一条 assistant 的合成推理文本原样放入 `reasoning_content`，工具调用与结果 ID 对应 | 工具续轮不能丢弃推理历史 |
| 固定 SDK 下一用户轮，关闭推理 | 发送 `thinking: {type: "disabled"}`，不发送 `reasoning_effort`；历史 assistant 保有 `reasoning_content` 字段 | 开关参数与历史保留是两项不同责任 |
| 固定 SDK 无推理块的 assistant 历史 | 补上合法的空串 `reasoning_content: ""` | 不可将空字段当成缺失或删去 |
| 固定 SDK 非推理模型调用 | 不发送 `thinking` 或 `reasoning_effort` | DeepSeek 格式分支依赖真实的 `model.reasoning` 条件 |
| 隔离 gateway 的两次直接 HTTP 诊断 | 首轮输入 `thinking` 在上游请求中消失；续轮 assistant 的 `reasoning_content` 消失，工具关联仍在；上游推理 SSE 均成为 terminal error，未向下游交付推理 delta | 缺口在请求、历史和流式响应三处；不是仅预览 guard 太严格 |

原合成 `models.json`、`auth.json` 和 `settings.json` 在完整执行后逐字节不变。SDK 输出预算按其 `simple-options.js` 的上下文估算和安全余量规则计算；本次只检查正确字段和正值，不把人为指定的 64 提升为 SDK 必须保持的预算合同。

直接 gateway 诊断绕过了导入，仅用于定位现有 sampling 编解码的丢失位置，不算正常导入验收通过。它使用与上述基线匹配的 release gateway 依赖，不加载用户应用配置。

## 缺口与后续边界

Pi 1.0.2 的 `pi-ai/dist/api/openai-completions.js` 在 DeepSeek 推理模型分支生成 `thinking` 开关；消息转换器保留带 `reasoning_content` signature 的推理文本，并在相应 assistant 历史缺少推理时补空串。当前 gateway 将 Chat Completions 输入转换为 sampling 的 text/tool-call 消息；该契约没有 reasoning 历史槽，provider 重建请求又不包含 `thinking`。当前 decoder 的允许字段也不包含 `reasoning_content`，所以实际响应不能完整回到 Pi。仅取消来源拒绝条件会把可见失败改成派发时的内容损失或终止错误。

[DeepSeek 官方 thinking mode 说明](https://api-docs.deepseek.com/guides/thinking_mode/)要求工具调用续轮回传完整 reasoning 内容。上述严格合成 oracle 验证这一边界，不以更改来源为 Responses 协议来绕过原 Chat Completions 缺口。

后续建议为同协议的原生 Chat Completions 操作，与现有原生 Responses 一样保留请求与 SSE 内容。Pi adapter 负责将固定 SDK 的有效兼容设置投影到受管 Pi 模型；AI-provider 负责协议传输，gateway 负责模型路由、认证和边界检查。这样无需把 Pi 厂商字段加入通用 sampling 消息。此处是基于证据的候选实施方向，本轮未实现或扩大协议源码。

## 人工重现

临时脚本为 [manual-pi-deepseek-import.py](../../scripts/manual-pi-deepseek-import.py)，不接入测试或 CI。生成的 Python 绑定须匹配 bundle 的 UniFFI API；路径参数必须为绝对路径。

```sh
python3 scripts/manual-pi-deepseek-import.py \
  --bundle /Applications/Velune.app \
  --bindings "$PWD/target/bindings/diagnostics-python" \
  --node "$(command -v node)" \
  --probe-deps "$PWD/target/release/deps" \
  --rustc "$(rustup which rustc)"
```

前面三个参数运行正常导入与 SDK 对照。最后两个参数一起提供时，脚本在隔离目录编译小型直接 gateway probe，只输出字段存在性、完整性断言结果及计数，不保存请求全文。结果中的 `normalImport.acceptance: "FAILED"` 是本基线的验收结论；**脚本执行成功不等于导入验收通过**。退出成功只表示成功复现并核对这些观察。该脚本目前固定核对已知失败基线，后续修复时应改为正常端到端成功的验收，或删除这份临时脚本；不将预期失败复现接入 CI。
