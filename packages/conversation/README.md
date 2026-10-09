# 会话契约

本 unit 是 Velune 内部会话投影的权威契约，定义运行时无关的会话摘要、快照、消息角色和有序类型内容、动作与通用设置描述。它不启动 Agent、不读取会话文件，不依赖 AI 提供商、网关、平台文件系统或 Pi envelope。

Pi 的事件解析和 projection 属 [agent-runtime](../agent-runtime/README.md)。真实历史由 Harness 持有，此包只定义 Velune 可替换的投影；远端工作目录也不意味着本机可访问路径。

会话 snapshot 的 `revision` 只描述当前 projection 生命周期，历史投影与新建的原生执行 projection 可分别起算。消费方不得据同一会话 ID 的较小 revision 丢弃准备后的新状态；平台通过自身请求顺序与视图 generation 防止旧回调覆盖新视图。此字段不构成跨阶段全局日志游标。

展示层可从消息内容派生多条稳定 UI 行，但这些行不成为另一份会话记录或领域事件日志。标题采用原生显式名称、第一条用户正文的单行摘要、未命名会话，文件名和 ID 不充当标题；适配器负责原生数据归一，时间以可选 Unix 毫秒表达，平台负责本地格式。

会话摘要保留所属实例、原生 CWD 和可选创建／更新时间（Unix 毫秒），缺失不编造。Mac 的全实例分组、筛选、排序是该摘要的展示投影，不改变会话执行身份。

跨运行时逻辑会话的摘要身份与所属实例表示首段来源；快照的 `context_runtime_id` 明确表示当前原生上下文，用于查询、取消与交互回复，不能由摘要来源推断。摘要的 `can_rename`／`can_delete` 由 application 结合原生段能力装配，本 package 不访问关联文件或替运行时执行管理。逻辑投影消息使用段命名空间，展示的交接边界不构成持久消息日志。

`Message.completed` 表达消息内容是否完成，不等同于执行成功。`TranscriptTurn` 是消息列表展示范围的轻量契约，仅引用用户、过程及外露正文的 canonical ID；`TranscriptItem` 给出完整有序展示项，大纲和消息身份映射使用稳定展示 ID。顺序投影实现位于 agent-runtime，平台不另建分组规则。展示记录不替代 canonical 消息，也不成为持久化 turn 数据库。

`MessageIdentityConfirmation` 仅表示当前投影生命周期内的 live 身份确认，重复消费必须幂等；它不是持久身份表。原生会话条目是最终身份来源。
