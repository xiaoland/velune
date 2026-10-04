# 开发与复验

项目不保留或新增自动化测试，以类型安全、静态检查和构建为先。行为验收使用手动操作或临时端到端脚本，不建立单元或集成测试套件，也不把临时脚本接入 CI。

## 当前 Mac 会话界面

当前应用为 SwiftUI 原生 Mac control surface：系统侧栏和工具栏、Chatbot 阅读与输入区、独立 Settings 窗口。系统决定基础字体、语义颜色及明暗外观，Velune 品牌仅保留在图标与少量细节。应用通过 UniFFI 0.32.2 生成的类型接口嵌入 Rust lib，消费 application 的具名用例，不解析 Harness 原生事件，也不启动常驻 Host 或访问 Unix socket。下文 IPC v1 与模拟体验段落描述历史原型，不能作为当前产品接入说明。

Mac 应用图标由 `scripts/render-app-icon.swift` 将 graphite 光学稿生成十档传统 ICNS 资源。Dock／应用库使用的图标应保留 macOS 的透明光学留白，不能将品牌稿直接铺满画布；具体绘制范围在生成器中维护，不能通过修改品牌 SVG 补偿平台外框尺寸。

安装固定 Pi runtime 后构建：

```sh
./scripts/install-pi-runtime.sh
bash scripts/build-macos.sh
open /Applications/Velune.app
```

应用根目录由启动进程的 `VELUNE_HOME` 环境变量指定，未设置或留空时为 `~/.velune`，非空值必须是绝对目录路径。平台将该路径显式传给 Application，库不自行读取环境。根目录下的 `generic-config.json` 持久化 AI 网关与 Agent 运行时实例配置，手动配置的 API key 留在 Keychain；导入的 API key 与受委托的订阅认证留在用户指定的原 Harness 存储，配置只保存来源引用。Finder 启动通常不继承 shell 配置，隔离开发可直接带环境运行 bundle 可执行文件：

```sh
VELUNE_HOME=/absolute/path/to/isolated-home /Applications/Velune.app/Contents/MacOS/Velune
```

构建脚本默认将验证签名后的 bundle 安装到 `/Applications/Velune.app`；仅构建可用 `bash scripts/build-macos.sh --build-only`。修改后退出应用再重建安装，安装器不会覆盖尚未退出的应用。用户已授权开发方随时直接退出 Velune；先正常退出，若应用拒绝退出，可终止已确认的 Velune 应用进程后安装，无需要求用户手动退出。产品显示版本来自仓库 `VERSION`，当前为 `0.1 beta.1`，原生“关于 Velune”面板读取同一版本。manifest 保存源码提交、UniFFI 版本、类型接口和配置 schema。

Settings 分为 AI 提供商、模型、模型路由和 Agent 运行时。先定义模型 ID、昵称、图标、上下文窗口、输出上限和支持的推理级别，再将模型关联到提供商的外部模型 ID，并显式选择路由。协议通过 Picker 选择，目前 OpenAI ChatCompletions v1 与 OpenAI Responses v1 可用；不进行协议翻译，未支持的协议不可保存。密钥由用户输入并存入本机 Keychain，文件只保存引用。fail-over 当前禁用，没有自动切换策略。

Agent 运行时可配置多个实例，每个实例选择类型、独立配置目录和初始模型；工作目录属于具体会话。首轮支持 Pi 类型，保持一个活跃 runner，空闲时切换实例。模型上下文窗口可留空保存草稿，但运行前必须填写正值，输出上限不能超过窗口。运行时默认模型用于新会话；会话中的模型切换由 Pi 保存，恢复时沿用该会话选择。连接后可新建或选择该实例的会话、发送消息、观察工具结果与取消。用户消息在右、LLM 在左、系统与工具状态居中，不显示作者头像或昵称。Pi 持久化会话，Mac 只投影。

Harness 仅收到 Velune 本机网关配置；提供商凭据不传入 Harness，不使用“工作环境已有认证”。正常调用由网关显式路由到配置的提供商。订阅来源可在提供商编辑页选择 Core 描述的运行时认证来源，指定原认证文件、Node 和认证提供商；“读取来源信息”读取非秘密元数据并填写协议与端点，“登录…”通过该来源的 SDK 展示原生交互。当前仅接入固定 Pi 新 `openai` 订阅路径，保留其 Pi 登录身份，不将 legacy `openai-codex` 认证接到公共 Responses。应用内登录成功或更换 API key 会更新认证绑定 generation 并要求重连；同一引用背后的外部账户替换尚未自动辨识，应经应用重新登录或明确更换来源。开发方不执行真实登录。真实请求和验收由用户完成。

AI 提供商页的“从运行时导入…”是完整配置导入入口。选择来源目录与 Node 后先读取预览，选择提供商模型，再导入；模型也可以关联到已有全局模型。原提供商与认证文件保留，配置只记录来源引用。重复项默认跳过，明确替换才更新已导入提供商；已有路由和运行时默认模型不自动改变。来源或目标配置在预览后变化时，须重新读取。仅支持当前网关能够保留语义的配置；不支持项及原因在预览中显示，动态命令不执行。

隔离视觉预览使用 `--preview`（合成多轮会话）或 `--preview-empty`；Apple app 不设置独立深色验收入口。预览 Store 无 Transport，不打开 Application、不访问真实配置、Keychain 或会话、不调用模型。dyld 加载惰性的库文件不等于打开运行时，预览不能证明真实循环完成。

## 历史无凭据核心原型

当前核心 0.1.0、诊断契约 1、SQLite schema 1。2026-10-02 在 Linux Codex Cloud 编译／执行。Rust 1.99.0（b940084d7）、rusqlite 0.37.0，全部依赖锁定于 Cargo.lock。不是 Mac 编译结果，也不是原生 Harness 协议兼容测试。

## 复现

官方 rustup 安装后，在仓库根目录运行；SQLite 通过 bundled C 源编译，需本机 C 工具链。首次下载 registry 依赖需要网络，运行原型本身不需要网络或任何凭据。

```sh
cargo fmt --check
cargo check --locked -p velune-host
cargo clippy --locked -p velune-host --all-targets -- -D warnings
cargo build --locked -p velune-host --release
cargo run --locked -p velune-host -- demo /tmp/velune-demo.sqlite
cargo run --locked -p velune-host -- inspect /tmp/velune-demo.sqlite
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

## 历史 Mac mini 模拟壳

2026-10-02 的模拟壳使用独立 Rust Host、Unix socket 和 SQLite。当前嵌入式应用已替代该接入，不再支持旧壳的运行与停止操作；`app/host/` 仅保留为独立的诊断程序。旧证据不能证明当前生成接口、真实 Harness 或模型调用已通过。


## Pi 运行时与本机网关

Rust agent-runtime 的 [Pi adapter](../packages/agent-runtime/src/client.rs) 处理显式 Node／CLI 路径与 JSONL RPC，同进程 application unit 负责运行时实例装配。会话列表使用固定 Pi 1.0.2 的 SDK helper，身份包含运行时实例，避免不同实例的同名会话混淆。RPC 没有 `list_sessions`；prompt 响应只表示接收，稳定终态为 `agent_settled`。取消先清队列再 abort，仍等待稳定终态后才允许关闭核心。

运行时配置保留入口、配置／状态根目录和可选会话存储目录，不要求工作目录。连接只准备实例网关与会话列表；新建时使用原生目录选择器指定项目目录，恢复时读取 Pi 保存的 cwd。列表覆盖该实例的多个项目，空闲切换重新启动 Pi child 并保留网关；已失效的目录明确报错，不能回退到应用启动目录。旧配置中的 `workingDir` 允许读取，但不再用于选择执行位置。

依据为 Pi `v1.0.2` 固定提交 `cd32f7725fdbddbaecdff5b1e68491563394e0ca` 的 [RPC](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/rpc.md)、[模型配置](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/models.md)与 [SDK 列表例子](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/examples/sdk/11-sessions.ts)。用户所说的 Pi home 由适配器映射为本版本的 `PI_CODING_AGENT_DIR`，不假定存在 `PI_HOME` 上游变量。

安装脚本固定 `@earendil-works/pi-coding-agent@1.0.2` 于忽略的 `target/pi-runtime`，不修改全局 npm 或 Pi。Mac bundle 将 SDK、CLI JavaScript 与 `packages/agent-runtime/resources/pi_sessions.mjs` 放入 Resources。Node 本体不随包，用户需指定 Node 22.19+ 的绝对路径，避免 Finder 启动依赖 shell PATH。CLI 入口为 `Contents/Resources/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js`。

Application 在实例专属目录生成受管 Pi 配置，只列出 Velune 网关端点、可路由模型与本地访问凭据；不写上游端点或 Keychain 引用，也不覆盖未受管的 `models.json`。恢复历史会话后重新绑定 Velune 网关模型，历史 provider 不能绕过网关。平台秘密 helper 在网关实际请求时解析 Keychain 引用或显式认证来源；来源 adapter 使用原存储锁刷新，不复制 refresh credential。开发验证不读取真实 Keychain 或来源文件。平台凭据 shim 只调用 Core 的 sealed 来源入口，具体 Harness 适配由 Core 选择；认证 helper 的 AuthStorage 文件入口绑定固定 Pi 1.0.2；版本不符明确拒绝，是依赖升级时须复核的边界。

执行 Pi 默认选择 `velune/auto`。Core 为本轮决定具体逻辑模型，virtual model 返回相同模型的能力；Pi 的物理模型 ID 使用非秘密路由绑定的稳定身份，网关将该 ID 直接映射到同一次已配置路由，不再二次选择；逻辑模型 ID 保留在界面与选择状态中。Pi 的分支 state 保存实际选择，assistant 历史记录实际模型；新会话使用运行时默认模型，会话切换不改这个默认值。模型 `maxTokens` 用作 Harness 元数据；输出参数仅按提供商协议发送和校验，订阅来源不支持服务端输出硬上限，不为其注入 `max_output_tokens`。Pi 的标准推理等级通过 `thinkingLevelMap` 限制为模型声明的等级；本轮不替自定义服务等级猜测转换规则。ChatCompletions 网关接收文本和工具消息，未支持的图片内容明确报错。Responses 保留原生请求 JSON、工具与 encrypted reasoning、JSON／SSE 响应；只支持 foreground 创建，不增加查询、删除或 background API。客户端断开或网关停止会释放活跃上游请求，上游失败不会发出成功的 `[DONE]`，也不自动重试或切换提供商。

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

## UniFFI 的临时端到端验收

先构建动态库，并为 `--resources` 提供包含 `pi_sessions.mjs`、`pi_virtual_model.mjs`、`pi_auth.mjs` 与固定 Pi `node_modules` 的资源目录。人工运行 [临时端到端脚本](../scripts/check-pi-uniffi-loop.py)，先按 [bindings unit](../packages/bindings/README.md) 生成 Python 绑定，再以绝对路径传入 `--bindings`、`--library`、`--resources` 和 `--node`。脚本使用临时 HOME、配置、工作与会话目录，以及 loopback 合成上游和 fixture-only 凭据 helper；不读取用户配置或调用真实模型。它检查流式回复、跨模型路由和输出上限、Pi 会话身份与恢复、忙时退出保护、取消以及配置重新打开。Responses 检查使用 `--protocol responsesV1`，订阅能力检查另加 `--subscription-capability`。用户对真实提供商和产品体验的验收仍独立进行。

## 工作区 units 与绑定构建

根 Cargo.toml 是虚拟工作区，不再输出 velune-core 产品库。共享单位的职责、接口和依赖在 [packages 各 README](../README.md) 与 [架构](design/architecture.md#独立-package-与平台装配) 中说明。Mac 构建会生成 Swift 源及 FFI modulemap，分别编译 Rust 绑定库与 Swift 语言模块，再链接原生 app；生成文件只放在 target 下。

```sh
cargo fmt --all --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo clippy --locked -p velune-bindings --no-default-features --all-targets -- -D warnings
cargo tree --locked -p velune-bindings --no-default-features
bash scripts/build-macos.sh
```

无默认 feature 的依赖树不含 agent-runtime、gateway、AI provider；本地会话操作明确返回 Unsupported。该构建只证明能力裁剪，不表示已支持远端操作。Kotlin 生成物依赖 JNA 及 kotlinx-coroutines；生成成功之外还需编译检查。C# 工具为第三方，UniFFI 版本兼容性仍需后续 C# app 接入时验证。
