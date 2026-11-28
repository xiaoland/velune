# Ark 工具续轮回放

`ark-glm-5.3-flash-pi-tool-loop.json` 是 2026-10-11 经 Pi 1.0.2 采集的 Ark Coding Plan 两轮 Chat Completions SSE 脱敏记录。原记录于 `6889cf7c7` 加入；旧手工脚本在 `9339fd3b7` 删除后，本 fixture 改由 `dispatch.rs` 的实际 gateway HTTP 测试消费。JSON 内保留采集版本、上游协议与脱敏范围。

场景仅读取合成 `fixture.py`，工具返回 `17 * 23` 的函数，下一轮回答 `RESULT=391`。测试检查 Messages 与 Chat 之间的工具调用、调用 ID、完整结果和结束语义。这是固定协议回放，不证明当前上游可用性、订阅额度或真实用户会话。
