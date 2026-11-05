# 意外同名 Codex 分支调查

用户看到多个 InKCre Dogfood，并明确未主动 fork。目标：解释真实创建原因；不能将 forked_from_id 等同用户意图，不能以加标签替代归因。

已确认：原会话01a1a55d...，11个原生子记录 forked_from_id 相同；多个沿用标题。时间11月4日11:43Z–13:34Z，cwd均dogfood-extensions/impl-py worktree，头部originator=Codex Desktop/source=vscode（可能继承，不单凭此断言创建者）。4份Velune逻辑关联未涉及这些分支。

范围：只读调查相关真实元数据与诊断，不读消息正文/凭据，不删除会话。root追本机日志与持久化索引，unexpected_fork_paths独立审查Velune触发路径。沿用svc-workflow方法，先核实因果再决定修复。

## 已确认根因

同一真实HOME只读thread/list对比：0.159.3返回78项，其中InKCre Dogfood原会话加4个attached线程；ChatGPT内置0.166返回74项，只保留原会话。差异恰为4个内部附属线程。证据 `target/verification/unexpected-codex-forks-20261105/list-comparison.jsonl`，仅保存所涉标题/身份/来源等元数据，没有turn内容。

原生166隔离schema中，ThreadListParams.attachedToThreadId文档明确省略时普通列表隐藏attached线程；ThreadForkParams.attachToSource明确普通fork与内部attached fork不同。159无这两个参数。此因果不是用户主动fork，也非migration复制数据；是旧CLI读取新版共享HOME时缺少原生可见性规则。

Velune源代码普通打开只thread/resume，重试同thread turn/start；thread/fork仅显式分叉、编辑选择fork及ephemeral侧聊，均未传attachToSource。当前159配置与159adapter彼此匹配，但不代表可以准确理解166已写入的共享HOME，这是此前升级核对漏项。

建议让此共享HOME使用现有166适配器及ChatGPT内置166CLI，保留实例身份和HOME；避免按同名、forked_from_id或sourceKinds粗略过滤，以免隐藏用户真正分支。当前调查未替换运行时配置或删除真实会话。若必须同时支持159读166HOME，需要额外的明确兼容方案，而不能声称其原生列表已经正确。
