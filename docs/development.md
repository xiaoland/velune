# 开发与复验

## 当前 Mac 会话界面

当前应用为 SwiftUI 原生 Mac control surface：系统侧栏和工具栏、Chatbot 阅读与输入区、独立 Settings 窗口。系统决定基础字体、语义颜色及明暗外观，Velune 品牌仅保留在图标与少量细节。应用使用 core 的通用 IPC v3 投影和配置描述，不解析 Harness 原生事件。下文 IPC v1 与模拟体验段落描述历史原型，不能作为当前界面操作说明。

安装固定 Pi runtime 后构建：

```sh
./scripts/install-pi-runtime.sh
bash scripts/build-macos.sh
open target/macos/Velune.app
```

应用根目录由启动进程的 `VELUNE_HOME` 环境变量指定，未设置或留空时为 `~/.velune`，非空值必须是绝对目录路径。根目录下的 `generic-config.json` 持久化 AI 网关与 Agent 运行时实例配置，凭据值仍留在 Keychain；本地 IPC 与运行文件使用同一根目录。Finder 启动通常不继承 shell 配置，隔离开发可直接带环境运行 bundle 可执行文件：

```sh
VELUNE_HOME=/absolute/path/to/isolated-home target/macos/Velune.app/Contents/MacOS/Velune
```

Settings 分为 AI 提供商、模型、模型路由和 Agent 运行时。先定义模型 ID、昵称、图标、输出上限和支持的推理级别，再将模型关联到提供商的外部模型 ID，并显式选择路由。协议通过 Picker 选择，目前仅 OpenAI ChatCompletions v1 可用；未支持的协议不可保存。密钥由用户输入并存入本机 Keychain，文件只保存引用。fail-over 当前禁用，没有自动切换策略。

Agent 运行时可配置多个实例，每个实例选择类型、独立配置目录、工作目录和初始模型。首轮支持 Pi 类型，保持一个活跃 runner，空闲时切换实例。连接后可新建或选择该实例的会话、发送消息、观察工具结果与取消。用户消息在右、LLM 在左、系统与工具状态居中，不显示作者头像或昵称。Pi 持久化会话，Mac 只投影。

Harness 仅收到 Velune 本机网关配置；提供商凭据不传入 Harness，不使用“工作环境已有认证”。正常调用由网关显式路由到配置的提供商。ChatGPT 订阅接入需要自己的正式认证／协议适配，不能通过 Pi 登录入口冒充已经接通。真实请求和验收由用户完成。

隔离视觉预览使用 `--preview`（合成多轮会话）或 `--preview-empty`；Apple app 不设置独立深色验收入口。预览 Store 无 Transport，不启动 Host，不访问真实配置、Keychain 或会话，不调用模型。预览不能证明真实循环完成。

## 历史无凭据核心原型

当前核心 0.1.0、诊断契约 1、SQLite schema 1。2026-10-02 在 Linux Codex Cloud 编译／执行。Rust 1.99.0（b940084d7）、rusqlite 0.37.0，全部依赖锁定于 Cargo.lock。不是 Mac 编译结果，也不是原生 Harness 协议兼容测试。

## 复现

官方 rustup 安装后，在仓库根目录运行；SQLite 通过 bundled C 源编译，需本机 C 工具链。首次下载 registry 依赖需要网络，运行原型本身不需要网络或任何凭据。

```sh
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
cargo run --locked -- demo /tmp/velune-demo.sqlite
cargo run --locked -- inspect /tmp/velune-demo.sqlite
```

demo 路径必须尚不存在，重复体验用新路径，不覆盖旧运行。inspect 要求已存在并取得独占连接；会增加 Run epoch、恢复未知投递，是启动／恢复命令，不是无副作用查询。

fixture 为 [review.json](../fixtures/review.json)，仅 `2 + 3 = 5`。Codex 会话发现 Claude Code／Pi，各委派一次独立检查；模拟 Harness 实际读取输入并生成结果，结果消息由 Codex 模拟 Harness 消费，然后独立验收。输出保存 tasks、attempts、events 与 `simulation:true`／`native_calls:0`。CLI demo 父任务保持 active；Host 的固定 fixture 验收器仅在两个子任务均 done 后完成合成父任务，不作为通用产品验收器。

## 已实现与没有实现

- SQLite：任务树、逻辑会话、不可变模拟 Segment、重启 Run、Attempt 的资源／账户／协议、消息状态与事件。事务提交注入意图后才调用适配器，结果与 outbox 同事务落库。
- 路由：明确资源 allowlist、协议匹配、模拟与资格过滤、账户粘性，输出 dispatch／wait／handoff_required／needs_decision；不把调度 Harness 当 LLM 路由。当前同协议候选按输入列表优先级决定。
- 协作：同 scope 发现与一层委派、幂等冲突检测、到期阻断、结果回传／消费和独立 fixture 验收。只有可信宿主 API，没有网络身份校验；不准把 caller ID 参数直接暴露给不可信模型。
- 故障：busy／确认未发送可在截止时间内由调用方重试；提交结果 unknown 不重放并阻塞同 Segment。重启保留队列、结果与任务；进程死亡不会凭空释放未知状态。无原生核对能力时保持 blocked，不能继续跑真实副作用。
- mock provider 直接产生确定性文本，Protocol 枚举只是路由兼容标签，未编解码 Responses／Messages 流。三 Native 边界均 `native_verified:false`，NativeUnavailable 显式报 unsupported。

未实现：真实模型/provider 调用验证、网络 gateway、MCP server、ACP、完整账户权益／费用预留／用量、通用权限／审批、运行中真实工具取消、自动 handoff、任意任务调度、数据库迁移、FFI／分发安装器。Pi RPC 与 SDK 会话列表已有隔离实现，但仍只按合成子进程验收。当前 Mac bundle 的构建与隔离验证结果见 [首循环任务](../tasks/pi-mac-first-loop/packet.md)。无真实登录、秘密读取、付费调用。模拟“费用为零”不推定真实未知用量为零。

## 原生协议参考

2026-10-02 复核以下官方页面与固定源码，仅作边界依据；没有安装对应版本。

| Harness | 未来适配边界 | 本轮证据 |
| --- | --- | --- |
| Codex | [app-server](https://learn.chatgpt.com/docs/app-server) 的 thread／turn；Responses provider；接纳和 turn 完成分开 | 页面复核；模拟执行，不是 wire 验证 |
| Claude Code | [官方 SDK sessions](https://code.claude.com/docs/en/agent-sdk/sessions)，不重写私有 CLI envelope；[Messages gateway](https://code.claude.com/docs/en/llm-gateway)；保留后台工作核对 | 页面复核；没有 SDK 桥或网关 |
| Pi | [v0.99.2 RPC](https://github.com/earendil-works/pi/blob/005af57d88ee23b33778f343a9595b32e67ff788/packages/coding-agent/docs/rpc.md) 行式命令；[固定 HEAD virtual-model route](https://github.com/earendil-works/pi/blob/e792ba131ed0495f3ff58a0eb13f20540e344d5c/packages/coding-agent/docs/virtual-models.md)；等待 settled | 固定文档复核；release 与 HEAD 不混用，未安装 |

下一步先安装并锁定官方 binary／SDK，用无秘密假上游验证 P1／P2／P12，再在获准账号与预算下联调。不能因为本轮模拟成功就勾选三 Harness 的全请求覆盖或真实跨会话消费。

## 历史真实联调研究

本节保留 2026-10-02 的研究背景。当前首循环遵循上文 Velune 网关路径，下面的 Harness 原生认证研究不构成当前应用的配置入口或已实现能力。

用户只需先提供不含秘密的配置选择：账号类别／计划、组织或 workspace 别名、允许的模型和数据目的地、订阅／API lane、额度与停止条件。不要发密码、token、auth 文件或 key 到聊天／仓库。

| 路径 | 用户需要准备 | 安全接入与尚待验证 |
| --- | --- | --- |
| Codex 原生订阅 | 有 Codex 使用权益的 ChatGPT 账户；两个账户保持独立；明确工作区／项目上下文可否流转 | 用户在 Mac 的官方 Codex 登录流程自行完成；[官方认证](https://learn.chatgpt.com/docs/auth)。独立配置及 keychain 隔离需实测，不复制已有缓存 token |
| 正式 ChatGPT 计划接入 | 应用用途、资格及注册选择 | [SIWC 官方入口](https://developers.openai.com/siwc/token-sharing-open-source)；本地／开源与商业托管资格分开。仅在另获授权后注册，不能借 CLI token 冒充正式接入 |
| Claude 原生订阅 | Pro／Max 或获邀 Teams／Enterprise 等有权账户 | 用户在未修改的官方 Claude Code 登录；[认证／独立配置目录](https://code.claude.com/docs/en/authentication)，Console 无 key 登录有隔离例外。遵守[托管与凭据边界](https://code.claude.com/docs/en/legal-and-compliance)，不代收／汇聚 Claude.ai token |
| API／云 provider | OpenAI／Anthropic API 项目或明确选定的云供应商及模型权限；不要求现在开通全部 | 获准后由用户经 Mac 系统秘密设施或执行环境 Secrets 配置，仅对子进程注入所需项，DB 只保留引用。现有原型不读取环境变量或 Secrets，也没有已验证的凭据集成 |
| Pi | 至少一个明确允许 Pi 使用的 provider／模型资源 | 按锁定 Pi 版本的官方登录或 provider key 路径配置独立 auth store；提供 OAuth 实现不证明供应商授权；沙箱与扩展信任另验 |

真实运行前必须具名批准：每次实验总货币上限、币种、单请求输入／输出限制、最大请求数／重试次数／总时长、并发、允许的资源与数据、是否允许付费 fallback。没有批准则真实调用上限为 0。建议首轮单 worker、仅合成输入，不运行变更型工具；达到任一上限立即停止新派发。401／资格拒绝不换号绕过；429 尊重范围及等待；未知费用保留保守预留，unknown 副作用停止重试。精确金额由用户决定，不能自行推定预算。原型尚没有实现这些真实费用闸门，必须在联调前补齐。

## 历史 Mac mini 模拟壳交付契约

用户指定 Mac mini 首个目标，后续明确同意先核心后 Swift＋AppKit 接线。父会话只读盘点报告：Apple Silicon arm64、macOS 15.4.1、Xcode 26.2、Swift 6.2.3、Rust/Cargo 1.93。当前固定工具链 1.99.0 需 Mac rustup 官方安装；尚未在本分支执行 Mac 编译，不能宣称已安装。

源码：[AppKit 薄壳](../app/mac/main.swift)、[构建脚本](../scripts/build-macos.sh)、[本地 Host](../app/host/src/host.rs)。原生壳启动独立 Rust Host，使用同用户私有目录（0700）内 Unix socket（0600）；不监听 TCP／LAN，不配置 launchagent 或 VPN。以随包 Rust `rpc` 子进程调用 IPC，Swift 不操作 DB。关闭 UI／客户端退出不终止 Host；安全停止取消未投递消息并关闭 Host，unknown 不伪装为已取消。再次启动恢复既有 DB。演示没有真实长运行工具，运行中工具取消尚未实现。

IPC v1 的命令为 `run/status/cancel/stop`，只允许内建合成样例，无任意 prompt／路径／账户参数；响应为 `{ok, diagnostics, host_error}` 或 `{ok:false,error}`。diagnostics 的 `contract_version=1` 与 tasks／sessions／messages／attempts／events 对应同一 SQLite 状态，显式 `simulation:true`。一个 Host 串行执行一条投递、每 900ms 前进一次以便观察，重启恢复待投递工作；非协议流速率。文件权限只是同用户本机边界，不抵御该用户自己的恶意进程，不能直接扩展到 LAN。

Mac 上复现（先停止旧版 Host，再重新构建）：

```sh
rustup toolchain install 1.99.0 --profile minimal --component rustfmt --component clippy
bash scripts/build-macos.sh
open target/macos/Velune.app
```

脚本只做本机 ad-hoc 签名 `codesign --sign -`，不选用户证书、不读签名秘密、不公证、不修改系统安装。产物在 `target/macos/Velune.app`，manifest 随包，二进制哈希在 `target/macos/binary-sha256.txt`。需 Python 3、Xcode CLT、cargo 在 PATH。此源码目标为本机调试，不是发布用签名包。

设备验收必须实际进行：

1. 启动 App，看到模拟标识／核心版本；运行样例，观察三会话与两次委派、路由 attempt、结果回传、验收完成。
2. 再运行，立即取消，确认未投递消息 cancelled，不产生新模型尝试；错误状态在状态栏可见。
3. 运行样例后立即退出 UI，几秒后重开，确认 Host 未被杀、任务继续完成、记录保留。
4. 点安全停止 Host，再连接／启动，确认 epoch 增长且历史保持；不要重复执行已消费消息。
5. 导出反馈 JSON，确认含构建版本与合成事件、不含输入 body／用户代码／秘密；文件只存本机，用户自行分享。

仍需父会话协调：将当前源码 commit／bundle 传到 Mac 的授权通道、实际运行构建命令、操作验收与用户反馈。已有设备构建使用授权；没有 push 授权，源码 bundle 仅为本轮 Cloud→Mac 传递，不替代 Git 仓库长期交付。iPad／Android／LAN 接入仍留后续。

每次设备构建交付一份 manifest：源码 commit、工作区是否 dirty、Cargo.lock 哈希、core_version／contract_version／schema_version、Rust／Xcode／macOS／目标架构、原生壳版本、构建命令、产物哈希和安装方法。JSON 诊断已提供核心／契约版本及 Run epoch；commit 与工具版本由构建脚本产生的 manifest 关联，不在当前 CLI 假造来源。

反馈最小模板：manifest 版本、设备／OS、步骤、期望／实际、错误类别、相关事件 sequence／message ID，可自愿附脱敏截图。不默认上传 DB、用户代码、消息 body、transcript、秘密或本机绝对路径。诊断仅对当前合成原型承诺无私有数据；真实接入后需另审数据字段。

闭环：Cloud 修复并做核心回归 → 新 commit／manifest → Mac 编译／签名／安装 → 重做原失败步骤和一次正常路径 → 用户确认。Task Packet 记录反馈 ID、修复 commit、设备验收结果；设备步骤未做则保持未验收，不能用接口测试替代。

## Pi 运行时与本机网关

Rust core 的 [Pi adapter](../core/src/pi.rs) 处理显式 Node／CLI 路径与 JSONL RPC，Host 负责运行时实例装配。会话列表使用固定 Pi 1.0.2 的 SDK helper，身份包含运行时实例，避免不同实例的同名会话混淆。RPC 没有 `list_sessions`；prompt 响应只表示接收，稳定终态为 `agent_settled`。取消先清队列再 abort，并停止网关中的活跃请求。

依据为 Pi `v1.0.2` 固定提交 `cd32f7725fdbddbaecdff5b1e68491563394e0ca` 的 [RPC](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/rpc.md)、[模型配置](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/models.md)与 [SDK 列表例子](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/examples/sdk/11-sessions.ts)。用户所说的 Pi home 由适配器映射为本版本的 `PI_CODING_AGENT_DIR`，不假定存在 `PI_HOME` 上游变量。

安装脚本固定 `@earendil-works/pi-coding-agent@1.0.2` 于忽略的 `target/pi-runtime`，不修改全局 npm 或 Pi。Mac bundle 将 SDK、CLI JavaScript 与 `core/pi_sessions.mjs` 放入 Resources。Node 本体不随包，用户需指定 Node 22.19+ 的绝对路径，避免 Finder 启动依赖 shell PATH。CLI 入口为 `Contents/Resources/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js`。

Host 在实例专属目录生成受管 Pi 配置，只列出 Velune 网关端点、可路由模型与本地访问凭据；不写上游端点或 Keychain 引用，也不覆盖未受管的 `models.json`。恢复历史会话后重新绑定 Velune 网关模型，历史 provider 不能绕过网关。平台秘密 helper 仅在网关实际请求时解析 Velune 自己的凭据引用，开发验证不执行真实 Keychain 读取。

选定模型的输出上限同时注入 Pi 的 `maxTokens` 与网关校验，避免 Pi 默认请求上限超过配置。Pi 的标准推理等级通过 `thinkingLevelMap` 限制为模型声明的等级；本轮不替自定义服务等级猜测转换规则。网关只接收文本内容与工具消息，未支持的图片内容明确报错。客户端断开或网关停止会释放活跃上游请求，上游失败不会发出成功的 `[DONE]`，也不自动重试或切换提供商。

隔离验证使用临时目录、fake RPC／假上游与合成工具结果，不启动真实用户会话或调用模型。正式验收状态与具体证据见 [首循环任务](../tasks/pi-mac-first-loop/packet.md)。

## 独立 AI service 的有界人工验收

此切片不运行上文旧原型的测试／Host／UI。先阅读 [AI service 契约](design/ai-service.md) 与 [当前任务及预算](../tasks/ai-service-contracts/packet.md)。Rust 1.99.0，仓库根目录：

```sh
cargo fmt --all --check
cargo check --locked --offline -p velune-ai -p velune-ai-provider --lib --example minimax_manual
cargo clippy --locked --offline -p velune-ai -p velune-ai-provider --lib --example minimax_manual -- -D warnings
cargo run --locked --offline -p velune-ai-provider --example minimax_manual -- replay text
cargo run --locked --offline -p velune-ai-provider --example minimax_manual -- replay text-diagnostic
cargo run --locked --offline -p velune-ai-provider --example minimax_manual -- replay tool
```

依赖尚未缓存时先允许 cargo 获取 Cargo.lock 固定包；`--offline` 只限制 cargo，不是操作系统网络隔离。replay 分支本身不构造网络 client 或读取凭据，输出 network_attempts=0。它比较独立审阅 expected，不执行模型或工具。重复运行同一命令核对相同事件／关联／分片／finish／usage。text 保留早期缺少终止证据的失败；text-diagnostic 按实际 clean EOF 记录与修正规则回放成功；tool 是修正后成功采集的真实工具调用。每个 expected 均独立审阅，未自动生成。

live 入口 `minimax_manual live text|text-diagnostic|tool SOURCE_COMMIT` 仅供已授权的两例及唯一一次诊断采集，必须在固定仓库根目录、干净且已提交源码上人工运行。Cloud 通过 Networksecret 提供 MINIMAX_API_KEY 占位；无需也不得把值写到文件、命令行或聊天。已存在的 admission／fixture 禁止覆盖；不能删除 admission 来重复计费采集。live 不作为日常构建或 CI 步骤，不自动重试。新的采集范围另行授权。

普通输出只列 case／attempts／typed usage／fixture 路径；正文与工具片段仅存获准的合成白名单 fixture。手动入口的本地 admission 文件不是防并发跨进程／跨目录绕过的全局产品预算服务，调用范围由本次授权和人工流程限定。
