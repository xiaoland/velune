# 原生上下文翻译交接

2026-11-04，用户授权落地并提交；不新增单元测试，使用合成原生会话与回环上游验收。

## 当前方案

早先任务记录中的“展示消息翻译／固定15万预算截断／尾段自包含”已作废。19:12前在途代码改为来源 adapter 导出 compact 后有效上下文，但应用调用未完成；重启后的收尾任务继续接线。相关决定从同项目的交接会话核对，用户要求 compact 语义可靠，不能丢摘要。

- 原生来源导出 → `import::Context` → 目标原生写入 → 既有恢复／发送。保留摘要、有效消息、工具参数及配对结果；不使用固定预算删除旧轮次。
- 展示继续读取封闭来源段；导入前缀按目标自身历史读取的消息数量与内容摘要定位，在界面隐藏，避免上下文副本重复展示。
- 导入先于关联，关联先于发送。无法解释的上下文在归属变化前报错。
- Codex 导入的 provider 与所选 native／managed 路径一致。
- ACP／DeepSeek 目标仍保留既有引用路径，原生导出暂不支持；旧引用载荷不可当作完整上下文重新导入。

## 验收状态

有效上下文版本已通过实际 Pi 1.4、Claude Code 2.1.292、Codex 0.166.0-alpha.6 与合成 HOME／回环上游验收：Pi compact 摘要、保留消息、工具参数与结果进入 Claude 请求，旧失效消息不进入；Claude→Pi 可继续；Codex 原生 compact 后→Claude 可继续，重开不重复展示。Codex 0.166 新的 world_state／token_usage_record／additional_tools 按元信息处理。未重复已通过的运行时路径；复用已 compact 的合成来源，只补失败交接。

Codex 没有完整 replacement_history 的继承／旧 compact，以及尚未解析的 rollback 显式拒绝；压缩 rollout 尚未接入。Pi 的展示沿用 SDK projection，它本身会隐藏 compact 前失效消息，不能承诺展示原生 SDK 不再显示的旧消息。原始取证及组合构建归 [工作区收尾任务](../workspace-delivery-and-network/packet.md)。
