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
- 已运行本地确定性模拟闭环；没有真实 Harness、账户登录或模型运行验证；ACPHub 精确源码尚不可访问
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

当前产品入口是下面的 Pi 与 Mac 会话切片；`app/host/` 只保留独立的历史诊断程序，不随 Mac 应用交付。

## 独立 AI service 契约

[AI service 设计与验收](docs/design/ai-service.md)是 2026-10-03 有界实现入口。`packages/ai` 拥有两套服务契约，`packages/ai-provider` 依赖它并提供 MiniMax Chat Completions 流式 adapter。直接派发 service 与合成 live fixture／离线 replay 独立于上述旧原型；本步仅静态检查和获准手动验收，不保留自动化测试。

## Pi 与 Mac 会话切片

共享能力在 `packages/` 中按 unit 拆分，平台 app 在 `app/`。AI、provider、会话契约、运行时和网关拥有各自 Rust 契约；application 装配完整用例并保存配置，bindings 生成 UniFFI 类型接口。平台 app 嵌入生成接口所对应的 Rust 库，负责原生界面和平台设施，不另启常驻 Host。Pi 持有会话历史，Velune 只投影列表、消息和运行状态。AI 提供商与全局模型目录由用户在应用内配置，不硬编码供应商示例；真实登录、运行与验收由用户完成。

网关支持原生 ChatCompletions v1 与 Responses v1，暂不翻译协议。Pi 会话选择稳定 `velune/auto`，实际路由使用绑定身份以保持历史兼容性。设置页支持手动配置，或从 Pi 预览并导入提供商、模型与认证来源；导入保留原存储，OAuth 登录／刷新委托来源 SDK；具体入口和限制见 [开发说明](docs/development.md)，隔离证据见当前 Task Packet。

Mac 构建默认安装到 `/Applications/Velune.app`，当前产品版本为 `0.1 beta.1`（归属 `VERSION`）；生成绑定契约与配置 schema 分开维护。安装器要求旧应用先退出；开发方按既有授权完成退出与重建安装。协作采用敏捷开发，需求可在实现中调整，关键设计先收敛再实施，具体规则见 [共享指引](AGENTS.md)。

本轮状态见 [首循环任务](tasks/pi-mac-first-loop/packet.md)，构建和操作入口见 [开发说明](docs/development.md)。不要将隔离协议检查或成功构建当成真实模型循环已通过。

各平台遵循所属平台的原生视觉与交互习惯；品牌在图标和少量细节中体现。Mac 使用系统侧栏、工具栏和独立设置窗口，具体原则见 [产品定义](docs/prd/index.md#已确认的产品结构与质量方向)。

共享 unit 入口：[ai](packages/ai/README.md)、[ai-provider](packages/ai-provider/README.md)、[conversation](packages/conversation/README.md)、[agent-runtime](packages/agent-runtime/README.md)、[gateway](packages/gateway/README.md)、[application](packages/application/README.md)、[bindings](packages/bindings/README.md)。平台入口：[Mac](app/mac/README.md)；本次拆分与验证见 [任务](tasks/package-boundaries/packet.md)。
