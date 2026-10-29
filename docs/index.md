# Velune 项目知识导航

这里是 Velune 长期知识的入口，每类知识只在一处维护。Beluna 是独立的姐妹项目，不与本导航混用。任务进行中的计划、临时证据与未决事项放在 Task Packet；任务完成后，仍成立的事实并入下列文档，任务包删除（git 保留历史）。

## 产品

- [产品定义](prd/index.md)：为什么做、核心优先级、已确认边界与术语；各能力的行为见其“能力”一节列出的分册
- [来源索引](sources.md)：关键用户决定的定位
- [界面与交互](../DESIGN.md)：跨平台的界面原则与交互模式

## 技术设计

- [Product TDD](product-tdd/index.md)：核心与平台 app 的分工，跨 unit 契约文档入口
- [多 Harness 路由与协作架构](design/architecture.md)：package 拆分与平台装配、会话权威与展示投影、运行时、协作与执行 owner
- [AI 服务与 AI 网关](design/ai-service.md)：操作、provider 与网关的职责，协议转换、路由、额度与用量分析
- [远程访问](design/remote-access.md)：设备、配对、连接与远程调用
- [遥测装配与关联](design/telemetry.md)：本机配置、SDK 代际、一次性分享与分布式诊断

各 unit 的内部设计在其 README：共享核心见 `packages/*/README.md`，平台 app 见 `app/*/README.md`。

## 开发

- [开发与复验](development.md)：构建、运行、验证方式与开发环境

## 当前工作

- [待决事项与已知缺口](../tasks/open-items/packet.md)：尚未解决的用户决定、未授权的候选与未验证的能力
- [侧聊](../tasks/side-chat/packet.md)：Claude Code `/btw` 式的侧聊，原生侧问或临时 fork（Mac）

## 协作入口

- [仓库 README](../README.md)
- [共享 Agent 指引](../AGENTS.md)

## 文档方法

采用 xiaoland/svc 的[知识归属](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/specs/index.md)与 [Task Packet](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/task-packet/index.md)方法。项目文档可以直接阅读和编辑，无需 SVC CLI、配置文件或运行环境。
