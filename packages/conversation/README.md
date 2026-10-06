# 会话契约

本 unit 定义运行时无关的会话摘要、快照、消息块、动作与通用设置描述。它不启动 Agent、不读取会话文件，不依赖 AI 提供商、网关、平台文件系统或 Pi envelope。

Pi 的事件解析和 projection 属 [agent-runtime](../agent-runtime/README.md)。真实历史由 Harness 持有，此包只定义 Velune 可替换的投影；远端工作目录也不意味着本机可访问路径。

会话 snapshot 的 `revision` 只描述当前 projection 生命周期，历史投影与新建的原生执行 projection 可分别起算。消费方不得据同一会话 ID 的较小 revision 丢弃准备后的新状态；平台通过自身请求顺序与视图 generation 防止旧回调覆盖新视图。此字段不构成跨阶段全局日志游标。
