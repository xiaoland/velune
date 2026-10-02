# 来源索引

这是关键决定的精简定位表，不是聊天记录导出。消息 ID 用于回到原对话核对；没有为它们编造可公开访问的 URL。未提供精确时间的消息不补猜时间。

## S1

- 消息：`Sentinel_14f3d0e763e08191a2024621ccb3e8c5`
- 时间：2026-10-01 08:47 UTC
- 决定：停止进一步 Factory26 开发，回到用户自己的 coding agent 产品

## S2

- 消息：`Sentinel_9f2c607f16748191a9a563bee396dacf`
- 时间：2026-10-01 08:49 UTC
- 决定：首要核心是多订阅、多模型供应商、自动路由、多 harness，以及跨 harness 会话感知、交流和协作；远程控制在此基础上通过分布式架构实现
- 纠正：此前远程／手机优先的讨论不再作为优先级依据

## S3

- 消息：`Sentinel_ce566f9e822881918fdfbebeedfcc75f`
- 决定：认可持久化任务—会话模型和任务树视图

## S4

- 消息：`Sentinel_06d6bb74003081919fbdccc180dfe423`
- 决定：HAPI／Lody 属说明性参考，不要求照搬其功能或实现

## S5

- 消息：`Sentinel_c03879c98b7881919e313d14c34d8c0a`
- 决定：Claude Code 订阅认证处理方式可以作为参考研究例外；尚未确定本产品认证架构

## S6

- 消息：`Sentinel_ce7bb3296a948191b830282c7b25b744`
- 决定：排除 GitHub Copilot

## S7

- 消息：`Sentinel_d84f6f77f16881919cce80d5d5ec86e4`
- 时间：2026-10-01 08:58 UTC
- 请求：先初始化 Git 仓库，包括 AGENTS.md、xiaoland/svc 文档知识系统和 Task Packet；完成后再继续讨论

## S8

- 消息：`Sentinel_fd6950f44cc48191b07f66cb0747c17f`
- 日期：2026-10-01
- 澄清：需要持久化仓库，不需要打包交付；SVC 只采用文档导航和 Task Packet，不安装或保留其 CLI 及额外工程
- 授权：可以使用用户明确给出的 Git 身份作本地提交；此授权不自动包含远端创建、push 或部署

## S9

- 消息：`Sentinel_55f5e7324b108191bc208034542f9f2b`
- 时间：2026-10-01 10:03 UTC
- 决定：首批 Harness 固定 Codex、Claude Code（语音中的 Cloud Code）、Pi；以全部可接管 LLM 路由为设计目标，调查如何实现，不以支持度重新开启选择
- 参考：跨 Harness 会话通信可研究 lexoliu/acphub，也可采用其他方案
- 委托：由设计方收敛方案，用户复核；本轮产出为仓库内方案与研究文档，不是产品实现

## S10

- 消息：`Sentinel_2ff262054c108191b25740ff1e05f8ba`
- 时间：2026-10-01 11:28 UTC
- 复核：认可上一轮四项产品设计；明确同意 MCP 暴露协作工具、ACP 作为可替换接入
- 纠正：保存任务树、消息、权限和恢复状态本身不能说明如何完成跨 Harness 协作，需要区分协调机制与支撑设施
- 技术方向：认可 SQLite，采用 Rust 核心；Apple 原生 Swift／UIKit、Android 原生 Kotlin／Jetpack Compose，替换 Web 前端；接受实验项目带来的开发不便
- 边界：未逐项批准 FFI 工具、Apple 平台覆盖／Mac Catalyst 选择、服务打包或所有详细实现契约；未授权产品实现、登录或费用测试

## 其他上下文的证据等级

Codex／Claude Code／Pi 的目标范围、LLM 路由与设备调度的区分、用户有两个 ChatGPT 账户，以及完整稳定可恢复、避免锁定等背景，来自初始化时传入的已整理上下文；当前没有为每一句提供独立消息 ID。本仓库保留其含义，不伪造逐句引文。若后来出现冲突，以最新用户明确决定为准并补充来源。

## SVC 官方来源

- [官方仓库](https://github.com/xiaoland/svc)
- 检查时间：2026-10-01 UTC
- 源提交：[4fe4c66ac4deb35209069c00b1bbdc1b22aae3af](https://github.com/xiaoland/svc/commit/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af)
- [Task Packet 语义](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/task-packet/index.md)
- [知识归属规范](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/specs/index.md)
- [PRD 规范](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/specs/prd/index.md)

仅采用上述文档方法，不把 SVC CLI 纳入本仓库。PRD 放在 `docs/prd/index.md`，遵从该提交的 PRD 规范入口；不另建旧模板路径的副本。
