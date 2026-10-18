# Velune

这是用户自有 Velune coding agent control surface 的独立仓库。Velune 提供控制界面与协调层，执行 Agent 由 Harness 承载。Velune 的名称已经确定；当前有无凭据 Rust＋SQLite 核心原型及产品知识。Beluna 是独立的、非 Coding 的通用 Agent / Digital Life 姐妹项目，不属于本仓库范围。

核心方向是多订阅、多 LLM 供应商、自动 LLM 路由、多 coding agent harness，以及跨 harness 会话的感知、交流和协作。远程、多设备和多平台能力是后续在此基础上的分布式扩展。

## 从这里继续

1. [AGENTS.md](AGENTS.md)：协作边界与文档更新规则
2. [产品定义](docs/prd/index.md)：已确认的目标、优先级、范围和术语
3. [架构方案](docs/design/architecture.md)：已认可的核心设计及 Rust／原生客户端细化建议
4. [当前 Task Packet](tasks/pi-mac-first-loop/packet.md)：Pi／Mac 实现状态、网关与配置修订、用户验收边界
5. [来源索引](docs/sources.md)：关键用户决定的消息 ID；不复制完整聊天

完整导航见 [docs/index.md](docs/index.md)。

## 当前状态

- `main` 已合并 MiniMax 流式 fixture 基线；Pi／Mac 首循环在独立开发分支持续迭代
- 按 xiaoland/svc 的知识归属和 Task Packet 方法组织文档，不依赖它的 CLI 或运行时
- 已建立共享 Agent 指引、产品知识与下一步 Task Packet，由 Git 记录版本
- 用户已认可核心产品设计、MCP／ACP 分工，以及 Rust＋SQLite、Apple／Android 原生 UI 方向
- 已按复核澄清实际协作机制与持久化支撑的区别；Apple 桌面、FFI、SDK 桥等具体边界仍待原型验证
- Pi、Codex app-server、DeepSeek Harness 与 Claude Code 已有版本化 Rust 适配器；原生／managed 模型、SSH 与受管 HOME 的共享核心和前端交付状态分别记录。账户登录、真实服务与 UI 体验由用户验收
- 当前仓库同时承载产品实现、权威长期文档和易变的任务状态

用户要求以持久化仓库承载后续讨论，不交付压缩包。远端地址为 <https://github.com/xiaoland/velune>；最初导入为重建文档快照，后续源码与文档由本仓库持续维护。

## 历史合成原型

使用官方 Rust 1.99.0 与本机 C 编译工具（SQLite bundled 需要）：

```sh
cargo run --locked -p velune-host -- demo /tmp/velune-demo.sqlite
cargo run --locked -p velune-host -- inspect /tmp/velune-demo.sqlite
```

`demo` 要求数据库不存在，输出 `simulation: true` 的 JSON：Codex 模拟会话向 Claude Code／Pi 模拟会话委派合成检查，结果返回 Codex 并验收。`inspect` 取得独占宿主后执行重启恢复并输出诊断，不是只读数据库工具。所有模型请求都在进程内确定性模拟；未启动真实 Harness、网络监听或模型 API。

完整构建、故障证据、Mac mini 原生体验接线及真实联调准备见 [开发说明](docs/development.md)。

当前产品入口是下面的 Agent 运行时与 Mac 会话切片；`app/host/` 只保留独立的历史诊断程序，不随 Mac 应用交付。

## 独立 AI service 契约

[AI service 设计与验收](docs/design/ai-service.md)是 2026-10-03 有界实现入口。`packages/ai` 拥有协议与 sampling 契约，`packages/ai-provider` 提供原生 ChatCompletions／Responses 执行。旧有界 MiniMax fixture 不是通用网关架构；当前需求与原生 HTTP 隔离验收归该设计，不保留自动化测试。

## Agent 运行时与 Mac 会话切片

共享能力在 `packages/` 中按 unit 拆分，平台 app 在 `app/`。AI、provider、会话契约、运行时和网关拥有各自 Rust 契约；application 装配完整用例并保存配置，bindings 生成 UniFFI 类型接口。平台 app 嵌入生成接口所对应的 Rust 库，负责原生界面和平台设施，不另启常驻 Host。各 Harness 持有会话历史，Velune 只投影列表、消息和运行状态。AI 提供商拥有可编辑模型条目，跨提供商共性用于参数模板快填；配置由用户在应用内维护，不硬编码供应商示例；真实登录、运行与验收由用户完成。

当前 LLM Gateway 是 AI 模块的一种应用模式，支持 ChatCompletions v1、Responses v1 与 Anthropic Messages：同协议原生透传，异协议采用 best-effort 请求、JSON 响应与增量 SSE 转换，不经过采样重建。它提供有序模型路由，在响应提交前遇到可切换失败时自动 fail-over；AI 能力也可由应用直接消费。当前转换实施与隔离验收见 [LLM 协议转换任务](tasks/llm-protocol-translation/packet.md)。Pi 会话选择稳定 `velune/auto`，实际路由使用绑定身份以保持历史兼容性。设置页支持手动配置，或从 Pi 预览并导入提供商、模型与认证来源；导入保留原存储，来源 OAuth 登录／刷新委托来源 SDK；Rust lib 另支持独立 Codex 订阅登录、续期与多订阅 fail-over，平台前端待另行接入（非正式 SIWC）；具体入口和限制见 [开发说明](docs/development.md)，隔离证据见当前 Task Packet。

Mac 构建默认安装到 `/Applications/Velune.app`，当前产品版本为 `0.1 beta.1`（归属 `VERSION`）；生成绑定契约与配置 schema 分开维护。安装器要求旧应用先退出；开发方按既有授权完成退出与重建安装。协作采用敏捷开发，需求可在实现中调整，关键设计先收敛再实施，具体规则见 [共享指引](AGENTS.md)。

当前 Rust 版本化运行时为 Pi 1.0、Codex 0.159、DeepSeek Harness 0.2、Claude Code 2.1，各自声明已验证的兼容版本区间（下限分别为 1.0.2、0.159.3、0.2.0-rc.2、2.1.286）；出现破坏性变化才使用独立适配器。Codex 通过原生 thread API 读取逻辑历史，DeepSeek 通过 huihua 发行包只读历史；原生 app-server／ACP 控制执行，不复制解析器源码或新建会话数据库。实施与安装状态见 [多运行时任务](tasks/multi-runtime/packet.md)，构建和操作入口见 [开发说明](docs/development.md)。不要将隔离协议检查或成功构建当成真实模型循环已通过。

跨 Harness 接续创建目标原生会话并交接可携带的文本上下文，Velune 只持久化原生引用和切换位置，消息仍由各 Harness 保存；统一逻辑会话可在重启后重新投影。不会重放工具执行或审批状态。当前能力与安装证据见 [接续任务](tasks/cross-harness-continuation/packet.md)。

各平台遵循所属平台的原生视觉与交互习惯；品牌在图标和少量细节中体现。Mac 使用系统侧栏、工具栏和独立设置窗口，具体原则见 [产品定义](docs/prd/index.md#已确认的产品结构与质量方向)。

共享品牌设计资源保存在 [assets/brand](assets/brand)，供各平台与项目材料使用。SVG 光学稿及其原始说明由 Git 跟踪；Mac 构建从此目录生成应用图标，平台产物不替代共享原稿。

共享 unit 入口：[ai](packages/ai/README.md)、[ai-provider](packages/ai-provider/README.md)、[conversation](packages/conversation/README.md)、[agent-runtime](packages/agent-runtime/README.md)、[gateway](packages/gateway/README.md)、[application-api](packages/application-api/README.md)、[remote](packages/remote/README.md)、[application](packages/application/README.md)、[bindings](packages/bindings/README.md)。平台入口：[Mac](app/mac/README.md)；拆分与验证见 [任务](tasks/package-boundaries/packet.md)。

远程控制通过 iroh 操作执行设备上的唯一应用核心，GUI 与 headless 使用相同 Rust 契约；remote-only UniFFI 控制端不携带 Harness／AI 执行栈。当前 [远控库任务](tasks/android-minimal/packet.md) 只交付 Rust Library、生成绑定和库级证据，已实现方向配对、稳定服务、统一寻址、自动恢复与设施设置。平台 App 尚未适配新契约，不能把本分支当作匹配产品安装；真实公网迁移与非默认双网仍未验证。完整契约归 [统一设备远控](docs/design/remote-access.md)，早期证据留在 [初版远控任务](tasks/remote-control/packet.md)。

2026-10-10 的 [运行时扩展](tasks/runtime-provider-ssh/packet.md) 仅推进 Rust packages：新增 Claude Code、运行时既有模型选择、单轮 Velune Gateway 覆盖、OpenSSH 执行和受管 HOME。composer 及 SSH／HOME 设置界面明确待实现，本轮没有重新构建或安装 Mac app。

通用 `acp-v1` 接入用于减少逐个适配 Harness 的维护成本：用户安装原生 ACP Agent 或社区 bridge 后，通过启动配置接入，按协商能力提供会话、模型选项与历史。既有专用适配器继续提供各 Harness 的网关与原生能力。Rust 实现和限定验收见 [ACP 任务](tasks/acp-adapter/packet.md)；App 入口待实现。

Harness 互联互通区分两种关系：Harness 自身的原生 sub-agent 属于会话体验；Agent 间协作（A2A）由 Velune 提供，任一会话中的 Agent 可经嵌入式 MCP 工具新建或联系其他运行时的普通会话、等待其回复并继续工作，参与方都出现在会话列表。操作契约见 [开发说明](docs/development.md#agent-间协作)，实施与 App 呈现见 [Agent 间协作任务](tasks/agent-collaboration/packet.md)。
