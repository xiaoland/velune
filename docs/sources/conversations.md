# 来源：会话管理与侧聊

[返回来源索引](../sources.md)。本页保留原始决定与当时的授权边界；它不是当前实现状态或本次任务的授权清单。现行规则从[文档入口](../index.md)按主题查阅。

## S24

2026-10-14 用户验收子 Agent 字段后，要求进一步改进会话管理：会话列表的分组体验、分页加载、总数限制、项目名称显示，以及会话归档。就助手的提问，用户选择：分组可折叠并记住折叠状态，组按最近活动排序；整个列表设总数上限，每组条数改为按需展开，没有“显示全部”，可以有“加载更多”；有的运行时可能有额外的项目注册信息，这样就不必读出全部原生会话也能按项目分组，并按需读取前 N 个会话；项目名称可自定义别名；归档原生优先，其余由 Velune 记录（Codex 用原生归档，Claude Code、Pi 等不改原生会话，已归档默认隐藏、可切换查看）。随后补充遗漏的四项：会话已读／未读、移动项目、复制深链／ID、置顶（Pin）。安装后用户指出 Codex 支持不指定工作目录的模式，本质是在 Codex 管理的位置（如 `~/Documents/Codex`）创建会话，建议将这些会话归为 Chats 或 No Project 组，其他 Agent 运行时有类似情况也这样处理。随后用户要求组旁不显示会话数量，并指出会话“运行中”时不应出现“未读”：未读本质上应定义为一个 turn 结束，不论正常还是异常，建议异常显示为感叹号标。产品归 [PRD](../prd/conversations.md) 会话浏览段落，实施归 会话列表管理任务（git 历史：tasks/conversation-list-management）。

## S29

2026-10-16 用户要求会话列表：点击项目名称也可以展开／收起组；条目不再显示最后更新日期和 Agent 运行时信息，只显示会话标题与状态。实现过程中，用户随即取消了“点击名称展开／收起组”，组标题保持原有行为。产品归 [PRD](../prd/conversations.md)，实施记录在 会话列表管理任务（git 历史：tasks/conversation-list-management）。

## S35

2026-10-17 用户发现 Claude Code 在 worktree 中的会话（如 `mac-sub-agents-panel`）各自成为独立项目。就开发方“worktree 是否归入所属仓库”的提问，用户答复：“是的，worktree 归到原项目下面；其实正好，我计划让 project 和特定的文件夹解耦，一个 project 可以有多个 folder，有一个 primary folder”。产品归 [PRD](../prd/conversations.md)，实施归 多文件夹项目任务（git 历史：tasks/project-folders）。

## S37

2026-10-18 用户要求：“使会话列表支持手动排序‘组’”。开发方的做法：拖动组标题或在组的右键菜单中移动，顺序随浏览偏好保存，项目与运行时分组各自一份；置顶组固定在最前，新出现的组排在已排序组之前，可恢复按最近活动排序。用户试用后确认可拖拽排序，并建议：“将这个作为一个显示的排序模式，从而不再依赖创建时间/更新时间，避免排序发生跳动”；据此改为显式的组排序方式（最近活动／手动），手动时新出现的组排在末尾。产品归 [PRD](../prd/conversations.md)，实施归 统一浏览任务（git 历史：tasks/conversation-browser）。

## S47

2026-10-23 用户要求“引入对 xurl 的支持”，确认对象为“Xuanwo/xurl：用 agents:// 读取、查询和继续各 Agent 的会话”。在会话地址互通、Agent 使用 xurl 与底层适配三个方向中答复：“1,2就好；3的话，如果有可以学习吸纳，改进我们的效率、稳定性、支持范围的，可以学习”。实现授权覆盖前两项与必要共享契约、隔离验证，不要求替换现有适配器；随后用户要求“给 windows 也接入，安卓也可以试一下”；Windows 验证明确使用 `win-ws.localhost` 的 SSH，Android 使用 ADB。实施归 xURL 任务（git 历史：tasks/xurl-support）。

用户进一步要求停止低收益边界测试并评估过度设计，随后明确：“开始精简；并且你实现的功能很奇怪，要手动复制 xurl 进行跳转...这有什么意义...我要 deeplink 那种”。此决定将用户入口收敛为点击链接后自动打开，仅在设备或实例有歧义时选择；不保留为跳转而新增的手动输入、查找和复制流程。

用户随后指定 Android 验证设备：“安卓用 huawei 去测试，不要用 redmi turbo3”。

2026-10-26 用户询问是否遗漏复制会话为 xURL；开发方建议恢复用于交给 Agent 或其他工具的地址导出，保留 Velune 链接，仅在核心提供有效原生地址时显示 xURL。用户答复：“请实现；不过这样的话 context menu 中‘拷贝’系列功能就有三个了，我建议把他们并入一个二级菜单下”。据此恢复 Mac／Windows 的地址导出并合并为“拷贝”子菜单，不恢复手动输入和查找页面。

## S56

2026-10-29 用户解除架构重构期间的功能冻结，并提出首个 beta 前要加的两项功能：“side-chat (aka. claude code's btw)”；“session's context menu provide abilities to interrupt/resume the session or even open the composer in a sheet (for one-time send)”。对侧聊的方案答复：上下文“优先使用 Agent Runtime 的原生行为，如果 Agent Runtime 本身没有那就是按照你说的选项1处理”（选项 1 为在运行时内原生 fork 出不保存的临时副本）；形态为多轮、关闭即丢弃；侧聊中能做什么“由 native agent runtime 自己处理，默认不做任何限制”；两项功能首个 beta 前只做 Mac。产品归 [会话与会话列表](../prd/conversations.md#侧聊) 与 [输入区](../prd/composer-actions.md#在-sheet-中发送)，侧聊的实施归 `tasks/side-chat`；会话菜单的实施已完成（git 历史：tasks/conversation-menu-actions）。

## S57

2026-10-30 用户要求侧聊在 UI 上复用普通会话：“它会 split the view（或者弹出 sheet，行为是可选的，就像 outline 那样）”，同时“它不是另外一种有着不同布局逻辑的 session”，允许背景颜色、标题和 hint 不同。产品归 [侧聊](../prd/conversations.md#侧聊)。

## S58

2026-10-30 用户进一步要求：会话列表右键菜单中的侧聊默认通过 sheet 打开，避免切换当前标签页；拆分发生在“editor”级别，而不是 editor 内部；简化 UI，不必说明 temporary，让它与普通会话只在名称上有所区分。产品归 [侧聊](../prd/conversations.md#侧聊)。同轮要求考虑撤销针对 SVC Dev 缺陷的脚本修复，PATH 继承交由 SVC Dev 上游处理；保留独立的构建与重装动作。
