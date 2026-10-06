# Velune 项目知识导航

这里沉淀 Velune 值得长期保留、已有明确归属的项目知识。Beluna 是独立的姐妹项目，不与本导航混用。跨单元方案放设计入口并标明复核状态；研究证据、试验缺口与当前工作状态保留在 Task Packet。

## 长期知识

- [产品定义](prd/index.md)：为什么做、核心优先级、已确认边界与术语
- [来源索引](sources.md)：关键用户决定和 SVC 文档方法来源

## 架构设计

- [多 Harness 路由与协作架构](design/architecture.md)：已认可产品方向及 Rust／SQLite／原生 UI 基线；具体协作机制、宿主／桥接推荐与验证边界

- [AI 服务与 AI 网关](design/ai-service.md)：已确认需求、操作／provider／网关职责、当前实现偏差及历史有界采样契约

## 当前工作

- [跨 Harness 会话接续](../tasks/cross-harness-continuation/packet.md)：目标原生会话、上下文交接、仅关联元数据与重启投影

- [配置体验与原生 Messages](../tasks/settings-protocol-refinement/packet.md)：历史诊断、原生多选与模板、运行时检测、按组分页及协议扩展

- [跨运行时统一会话浏览](../tasks/conversation-browser/packet.md)：全启用实例、项目与标签分组筛选、创建／更新时间排序

- [原生会话管理与切换加载](../tasks/session-management/packet.md)：真实重命名／删除、加载目标与已加载内容隔离

- [运行时与会话配置体验复核](../tasks/runtime-session-experience/packet.md)：会话模型与自动执行准备、独立历史浏览、公开目录模板和原生菜单修复

- [Magpie 架构参考复核](../tasks/magpie-reference/packet.md)：配置 adapter、模型目录、原生／兼容协议与可观测性；已吸纳版本能力、导入边界及无正文网关观测

- [版本化 Codex 与 DeepSeek 运行时](../tasks/multi-runtime/packet.md)：原生控制、huihua 包历史投影、许可与安装包隔离验收；GUI／真实服务由用户验收

- [AI 网关与 AI 服务需求复核及审计](../tasks/ai-gateway-audit/packet.md)：原生协议保真、路由／fail-over、配置／认证和生命周期，包含源码证据与迁移建议

- [独立 package 与平台能力装配](../tasks/package-boundaries/packet.md)：已授权并完成 UniFFI 拆分，保留其边界与验证证据

- [用量与性能分析](../tasks/usage-analytics/packet.md)：网关用量事实、统计口径、原生分析界面与隔离验收

- [Pi 与 Mac 首循环](../tasks/pi-mac-first-loop/packet.md)：原生 control surface、AI 网关、提供商与模型配置、Agent 运行时实例；真实运行待用户验收

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

- [会话模型与消息展示](../tasks/conversation-presentation/packet.md)：标题与时间归一、权威投影、原生消息呈现和长列表
