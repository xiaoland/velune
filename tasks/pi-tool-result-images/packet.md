# Pi 工具结果图片投影

## 目标

Pi 会话中 `read` 等工具返回的 `toolResult` 带 `type:"image"` content 块（base64 + mimeType）。Pi TUI 内联展示这些图片，但 Velune 消息列表看不到，因为 `packages/agent-runtime/src/projection.rs` 的 `apply_tool_result` 只提取文本（`text_content`），图片块被丢弃。已获用户授权修复。

## 方案

在 `apply_tool_result` 中把 content 数组里的 image 块投影为 `MessageBlock::Image`，挂到持有对应 `toolCall` 块的消息上（通常是 assistant 消息）。交付链（`delivery.rs` → `DeliveredBlock::Image`）与 UI（`RenderedMessage` 对 image 块生成 attachment，不分 role）已支持，无需改 UI。

关键约束：

- `message_start`/`message_end` 会对同一 toolResult 各调一次 `apply_tool_result`，append 必须按 block 内容去重（幂等）。
- 一个 assistant 消息可能有多个并行 toolCall，attach 时只碰 tool_id 匹配的消息。
- 不改 `MessageBlock` 契约；工作区有其他任务未提交改动，只改 `projection.rs`。

## 验收

- `cargo test -p velune-agent-runtime` projection 相关测试通过，新增测试覆盖：toolResult 图片挂到 toolCall 消息；start/end 双事件不产生重复 Image 块；replace_history 路径。
- clippy / fmt 通过。

## 当前状态

已提交（898748c）。实现见 `projection.rs`：`image_blocks`/`attach_tool_images`，`apply_tool_result` 挂 Image 块并按内容去重保证 start/end 双事件幂等。3 个新测试通过（历史替换路径附图、start/end 双事件幂等、无调用方时独立 Tool 消息），用户在 Mac 原生关键路径验证图片正常显示。已确认其他投影消费方（transcript/collaborators/sub_agents/delivery）按块类型匹配，附加 Image 块无副作用。
