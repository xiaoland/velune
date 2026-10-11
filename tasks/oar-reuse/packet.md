# OAR 复用评估

2026-10-11 用户要求评估 `botiverse/oar` 是否值得复用，动机是减少逐 Harness 适配和维护成本。授权范围为调查、隔离实验和任务记录；没有授权切换实现、引入依赖或修改 App。保留 Rust 同进程核心、UniFFI、原生会话归属、SSH／managed HOME 与现有协作授权边界，不读取真实认证或会话，不调用真实模型。

核对固定提交的公开接口和实现，对比原生会话控制、持久资源 CRUD、跨 Harness 子 Agent、提供商注入及执行位置。先找能实际替代 Velune 代码的复用单元，再判断 Node／Rust 边界和生命周期成本。结束时给出可采用的建议、判别证据和仍未验证的部分；未提交的 `app/mac/.DS_Store` 与 `app/mac/Assets/` 属于其它工作，不触碰。

## 固定来源与观察

评估固定提交 `67d77ddec470dda5add664644c0cfb29c679660c`（2026-10-11），源码 package 版本 `0.56.0`。已完成公开源码核对和独立 advisor 判断，没有安装或执行 OAR，没有跑其自动化测试或 native Harness。结论属于接入评估，不是运行兼容认证。

[package.json](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/packages/oar/package.json) 声明 TypeScript／ESM、Node 24+、Apache-2.0，以及 Pi SDK `^1.1.0`；Velune 当前专用 Pi variant 为 1.0.2，不能把两者当成同版本 SDK。许可证正文见 [LICENSE](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/LICENSE)。当前不是以语言或既有开发投入否决复用：OAR 的上游适配维护、原生事件解码、steer／queue／abort、native child attribution 和模型／MCP 发现有实际价值。

| 范围 | 源码事实与对 Velune 的影响 |
| --- | --- |
| 原生会话适配 | Runtime／Session 提供启动、prompt、steer、queue、abort、dispose 和 record／event stream，具备替代部分 Codex／Claude 协议处理的可能。需要一个由 Rust 管理的 Node worker 和事件映射，不能直接经 UniFFI 导入 TS。每实例 worker 可以沿现有 Pi helper 边界，不新增常驻 Host。 |
| 宿主交互 | Session 没有原生 request reply 公共方法。Claude 始终带 `--dangerously-skip-permissions`；Codex 使用 `approvalPolicy: never`，收到 user input／dynamic tools 等 server request 只记录而不答；ACP 自动选 `allow_always`／`allow_once`；Pi 开会话前向 trust store 持久写入项目信任。Velune 的审批和用户问题无法仅靠外层事件过滤保留。 |
| 原生资源管理 | inventory 只提供 skills／commands／MCP／tools 发现摘要，没有 source bytes、完整 assets、expected revision 或 CRUD。发现可以启动 native process／SDK、加载扩展或连接 MCP，不能替代本轮不启动 Harness 的资源管理查询。 |
| 跨 Harness 协作 | createSubagents 和 CLI `oar mcp` 可跨 runtime 执行，但 spawn 可指定 cwd／model／env，并使用各 adapter 的全权限默认策略；不是 Velune 的固定 target grants、宿主审批和父子生命周期契约。接入会话 adapter 时仍应使用 application 现有协调器，避免维护两套 child 状态。 |
| ACP、历史、执行位置 | OAR 明确拒绝第三方 bridge，公开 registry 没有 generic ACP；Runtime 契约未提供通用原生会话列表／历史读取，resume 是已知 ID 的继续执行。未发现公开 SSH execution target／transport factory。Velune 的任意命令 ACP、native history 和 SSH 管理仍需保留；远端可运行 Node worker，但远端分发、隧道、路径和清理需自行接线。 |
| 网关与多实例 HOME | 子进程 runtime 的 env／launchArgs 可承载部分原生配置，但不是每 turn 网关覆盖、终态释放和原生恢复契约。Pi 的 env 只影响 tool 子进程，SDK 配置根取进程级 `OAR_PI_AGENT_DIR`，且会话没有临时 ModelRuntime provider 注入钩子。每实例 worker 可解决环境隔离，不能直接解决 provider 注入或 SDK 版本选择。 |

具体来源：[Session 契约](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/packages/oar/src/contracts/session.ts)、[会话选项](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/packages/oar/src/contracts/session-options.ts)、[Claude 启动](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/packages/oar/src/runtimes/claude/launch.ts)、[Codex 会话](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/packages/oar/src/runtimes/codex/session.ts)、[ACP 权限回复](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/packages/oar/src/shared/acp/client-app.ts)、[Pi 开会话](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/packages/oar/src/runtimes/pi/open.ts)、[资源发现](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/docs/spec/inventory.md)、[子 Agent 契约](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/docs/spec/subagents.md)、[第三方 bridge 决定](https://github.com/botiverse/oar/blob/67d77ddec470dda5add664644c0cfb29c679660c/docs/design/decisions.md#reaching-a-runtime-through-another-hosts-adapter-2026-10-05)。

## 建议与下一步判别

当前建议是把 OAR 作为原生 Session adapter 的复用候选，暂不整包接入生产或迁移已有适配器。Rust application／UniFFI、资源 CRUD、通用 ACP、原生历史、SSH、网关和固定授权协调器保留。取得兼容的宿主控制契约后，可以真正复用其适配代码并删除相应自维护协议处理；仅移植源码会继续由 Velune 承担上游变化，不能计为消除了维护成本。

第一项条件是上游提供可选权限策略与 native request／reply／取消关联；Pi 另需明确会话 SDK／provider 装配入口。长期私有 fork 需逐 adapter 跟进，可能抵消复用收益。调查没有联系上游、发布 issue 或承诺其会接受变更。

最有辨别力的后续实验是仅使用未修改 OAR 的公开 API，以隔离 synthetic Codex peer 发出审批和用户问题，尝试贯通“收到请求 → 宿主 reply → 原生继续 → cancel／dispose”，并确认原生 HOME 无额外持久修改。目前公共契约缺 reply，预计需要上游扩展；本任务没有为了完成评估先安装依赖或实现 wrapper。契约具备后，再对单个 Codex Node worker 验证原生／managed 两轮、取消清理和 SSH，成功才扩大替代范围。

评估完成；没有产品架构决定或源码迁移，相关建议仍待用户采用。仅记录此任务结论，不修改 PRD 或 architecture 使候选变成已确认方案。
