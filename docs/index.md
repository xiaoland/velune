# Velune 项目知识导航

这里沉淀 Velune 值得长期保留、已有明确归属的项目知识。Beluna 是独立的姐妹项目，不与本导航混用。跨单元方案放设计入口并标明复核状态；研究证据、试验缺口与当前工作状态保留在 Task Packet。

## 长期知识

- [产品定义](prd/index.md)：为什么做、核心优先级、已确认边界与术语
- [来源索引](sources.md)：关键用户决定和 SVC 文档方法来源

## 架构设计

- [多 Harness 路由与协作架构](design/architecture.md)：已认可产品方向及 Rust／SQLite／原生 UI 基线；具体协作机制、宿主／桥接推荐与验证边界

- [AI service 契约与 provider 配置](design/ai-service.md)：2026-10-03 独立 lib 的权威归属、静态保护及验收边界

## 当前工作

- [AI service 有界实现](../tasks/ai-service-contracts/packet.md)：隔离旧改动、契约审核、真实接入／fixture 待确认项

- [Harness、LLM 路由与跨会话协作](../tasks/harness-routing-feasibility/packet.md)：无凭据核心已实现；真实 Harness 与设备体验待接线
- [证据快照](../tasks/harness-routing-feasibility/evidence.md)：源版本、官方链接、事实与适用边界

## 协作入口

- [仓库 README](../README.md)
- [共享 Agent 指引](../AGENTS.md)

已授权并实现无凭据核心；真实模型与原生设备体验分别验收。

- [开发与复验](development.md)：运行命令、能力边界、Mac mini 接线、真实联调准备与反馈契约

## 文档方法

采用 xiaoland/svc 的[知识归属](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/specs/index.md)与 [Task Packet](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/task-packet/index.md)方法。项目文档可以直接阅读和编辑，无需 SVC CLI、配置文件或运行环境。
