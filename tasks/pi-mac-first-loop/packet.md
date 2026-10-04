# Pi 与 Mac 首循环

## 目标与当前范围

交付 Velune 原生 Mac control surface：会话列表与聊天界面、应用配置、Agent 运行时实例，以及 `Pi → Velune AI 服务网关 → AI 提供商` 的实际调用路径。开发方完成构建与隔离验证，真实登录、模型请求和最终验收由用户执行；不读取真实凭据或会话，不调用真实模型。

长期决定归属 [PRD](../../docs/prd/index.md)、[架构](../../docs/design/architecture.md) 与 [AI service](../../docs/design/ai-service.md)，运行入口归属 [开发说明](../../docs/development.md)。用户已授权实施、自主提交当前任务，并要求持续维护任务、长期文档与代码可维护性；远端发布仍需独立授权。

## 已确认的边界

- Velune 是 control surface。消息不显示头像或昵称，用户在右、LLM 在左、系统／Harness 在中间，不以 Velune 标记 LLM 作者。
- 各平台遵循原生视觉与交互；品牌仅在少量细节体现，默认 graphite logo。Apple 不设独立深色模式开发或验收项。
- core 拥有 AI 服务与 Harness 适配；app 拥有平台 UI、Host 与秘密设施装配。Mac 不解析 Pi 协议或按提供商分支。
- 模型跨提供商独立存在，有 ID、昵称、图标、输出上限与推理级别；提供商通过外部模型 ID 关联多个模型。协议是有限选项，首先支持 OpenAI ChatCompletions v1。
- 提供商、模型路由和 fail-over 归属网关配置。首轮使用显式路由，fail-over 禁用，不擅自增加自动策略。Harness 只接 Velune 网关，不利用其既有上游认证。
- Agent 运行时区分类型和实例。同为 Pi、不同配置目录或工作目录形成不同实例。首轮一个活跃 runner，空闲切换实例，会话身份按实例隔离。
- 应用配置文件位于 `VELUNE_HOME`，默认 `~/.velune`。凭据值在 Keychain，文件仅保存引用。Pi 拥有会话持久化，Velune 只做 projection。

## 基线与依赖

`main` 已从 `dev/minimax-stream-fixtures` 快进至 `2684f14755bfd1e41d36d48ddbb4d68afaa0fb85`，本地和远端 main 已同步。当前开发分支 `feat/pi-mac-first-loop` 承载本切片，尚未发布。旧 MiniMax 合成 fixture 人工 replay 已匹配 expected，不能代替新网关证据。

Pi 固定 1.0.2、commit `cd32f7725fdbddbaecdff5b1e68491563394e0ca`，SDK／CLI 安装于隔离 target 并随 bundle 放入 Resources；Node 22.19+ 由用户配置。该版没有 list_sessions RPC，列表通过 [SessionManager.list](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/examples/sdk/11-sessions.ts)；稳定终态为 agent_settled。Pi home 映射为本版 PI_CODING_AGENT_DIR。依据见 [RPC](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/rpc.md)与 [模型配置](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/models.md)。

## 当前实现与证据

源码已实际拆为 core／app。Mac 已用 SwiftUI WindowGroup／Settings、NavigationSplitView／List、系统工具栏与 Form 重写，graphite 保持原光学几何。隔离 preview bundle 强制内存 Store，不创建 Transport 或访问真实配置；已观察消息左右布局、独立提供商／模型表单、协议选项、多实例列表、较小窗口及合成快捷键发送。此前深色截图仅为历史观察，不进入后续验收。

配置编辑成功才关闭表单，失败保持输入与稳定草稿 ID；pending 禁止重复保存。Swift fake Host 检查已证明拒绝保存不触发成功回调、需重新连接时清空旧投影、旧 Host 版本不匹配时明确提示，不自动杀用户工作。新配置与实例契约使用 IPC 3，配置 schema 2。

2026-10-04 用固定 Pi 1.0.2、独立临时配置／工作／会话目录与 loopback fake ChatCompletions 服务捕获了一次真实 wire 请求：用户 content 为 text parts 数组，包含 tools、max_completion_tokens、store 与 stream_options.include_usage。合成 SSE 文本和 usage 被 Pi 正确解析；未访问外网、真实配置或凭据，未完成工具往返。网关必须规范化该真实输入形状，并明确处理不支持的内容，而不能只接受字符串或悄悄忽略参数。

当前七文件 Swift 固定快照编译无警告，默认原生外观的隔离预览已检查消息布局、居中工具事件、graphite 空态与多实例设置。实际 IPC 3 检查使用临时 Host 配置与 fake runtime，证明第二实例选择、模型昵称投影、路由保存后断连、跨两次重启保留提供商映射／实例目录／路由。未执行 Keychain helper 或模型派发。

Rust 网关已通过合成上游的外部模型选择、输出／推理参数与工具结果请求检查，以及非法输入 HTTP 错误、未路由草稿不阻断已路由模型、客户端断开和 Runner drop 及时释放无响应上游、上游 500 不产生成功 `[DONE]`。Pi 目录注入输出上限与标准推理等级限制。`cargo fmt --check`、`cargo check --locked`、workspace clippy（warnings 视为错误）、全量 `cargo test --locked` 均通过。本轮整包构建与 ad-hoc deep／strict 签名通过；提交后重建，使最终 manifest 关联当前提交与干净工作区。

固定真实 Pi 的完整隔离首轮已通过：临时 Host bundle、独立 Pi 配置／会话／工作目录、fixture-only 凭据 helper 与 loopback fake 上游。IPC 3 配置→连接→创建→发送成功；内部模型 m 被网关映射为 upstream-m，模型输出上限 64 进入实际上游请求，合成 SSE 返回后投影含 user／assistant 文本、modelId=m，状态回到 idle 且 canSend=true。首次实验使用旧 debug 产物失败；重新构建当前 Host 后通过。因此最终交付必须用 manifest 关联源码提交，不能复用旧二进制。

## 完成条件与用户验收

开发验证须证明合成请求经过真实网关并选中正确提供商，输出上限／推理级别与工具结果往返贯通，失败不产生成功终态，取消停止活跃上游请求；新配置和多运行时实例可重启恢复。最终 bundle 以 manifest 关联当前提交，保留 native_verified=false，不能因隔离验证将任务关闭为真实循环已通过。

用户随后在应用配置模型、提供商与显式路由，配置并选择运行时实例，选择／新建会话，发送消息，观察流与工具完成，取消，并重开继续观察。ChatGPT 订阅认证与未实现协议不伪装为已支持；资源例子不成为硬编码预设。
