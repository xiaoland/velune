# Velune 项目知识导航

这里沉淀 Velune 值得长期保留、已有明确归属的项目知识。Beluna 是独立的姐妹项目，不与本导航混用。跨单元方案放设计入口并标明复核状态；研究证据、试验缺口与当前工作状态保留在 Task Packet。

## 长期知识

- [产品定义](prd/index.md)：为什么做、核心优先级、已确认边界与术语
- [来源索引](sources.md)：关键用户决定和 SVC 文档方法来源

## 架构设计

- [多 Harness 路由与协作架构](design/architecture.md)：已认可产品方向及 Rust／SQLite／原生 UI 基线；具体协作机制、宿主／桥接推荐与验证边界

## 当前工作

- [Harness、LLM 路由与跨会话协作](../tasks/harness-routing-feasibility/packet.md)：核心设计已复核，细化与原型队列已更新；无实现
- [证据快照](../tasks/harness-routing-feasibility/evidence.md)：源版本、官方链接、事实与适用边界

## 协作入口

- [仓库 README](../README.md)
- [共享 Agent 指引](../AGENTS.md)

技术方案已有实际内容，但尚未获准实施；不建立空的单元 TDD 或部署规范。

## 文档方法

采用 xiaoland/svc 的[知识归属](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/specs/index.md)与 [Task Packet](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/task-packet/index.md)方法。项目文档可以直接阅读和编辑，无需 SVC CLI、配置文件或运行环境。
