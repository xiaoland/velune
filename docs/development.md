# 开发与复验

项目不保留或新增自动化测试，以类型安全、静态检查和构建为先。行为验收使用手动操作或临时端到端脚本，不建立单元或集成测试套件，也不把临时脚本接入 CI。

## 当前 Mac 会话界面

当前应用为 SwiftUI 原生 Mac control surface：系统侧栏和工具栏、Chatbot 阅读与输入区、独立 Settings 窗口。系统决定基础字体、语义颜色及明暗外观，Velune 品牌仅保留在图标与少量细节。应用通过 UniFFI 0.32.2 生成的类型接口嵌入 Rust lib，消费 application 的具名用例，不解析 Harness 原生事件，也不启动常驻 Host 或访问 Unix socket。下文 IPC v1 与模拟体验段落描述历史原型，不能作为当前产品接入说明。

Mac 应用图标由 `scripts/render-app-icon.swift` 将 graphite 光学稿生成十档传统 ICNS 资源。Dock／应用库使用的图标应保留 macOS 的透明光学留白，不能将品牌稿直接铺满画布；具体绘制范围在生成器中维护，不能通过修改品牌 SVG 补偿平台外框尺寸。

Mac 使用根目录 SwiftPM manifest 与 Package.resolved 锁定 MarkdownUI 及传递依赖；构建脚本先生成 UniFFI 模块，再构建原生 SwiftPM 产品，固定 macOS 14 部署底线并随包保留 Swift 许可证。不会构建或执行依赖的测试 targets。安装历史读取依赖后构建：

```sh
./scripts/install-runtime-support.sh
bash scripts/build-macos.sh
open /Applications/Velune.app
```

应用根目录由启动进程的 `VELUNE_HOME` 环境变量指定，未设置或留空时为 `~/.velune`，非空值必须是绝对目录路径。平台将该路径显式传给 Application，库不自行读取环境。根目录下的 `generic-config.json` 持久化 AI 网关与 Agent 运行时实例配置，API key 由提供商私有认证配置保存，文件权限为 0600；订阅仍委托原 Harness 登录与刷新。公开列表不含 key，查看编辑通过提供商内显式操作。Finder 启动通常不继承 shell 配置，隔离开发可直接带环境运行 bundle 可执行文件：

```sh
VELUNE_HOME=/absolute/path/to/isolated-home /Applications/Velune.app/Contents/MacOS/Velune
```

构建脚本默认将验证签名后的 bundle 安装到 `/Applications/Velune.app`；仅构建可用 `bash scripts/build-macos.sh --build-only`。修改后退出应用再重建安装，安装器不会覆盖尚未退出的应用。用户已授权开发方随时直接退出 Velune；先正常退出，若应用拒绝退出，可终止已确认的 Velune 应用进程后安装，无需要求用户手动退出。产品显示版本来自仓库 `VERSION`，当前为 `0.1 beta.1`，原生“关于 Velune”面板读取同一版本。manifest 保存源码提交、UniFFI 版本、类型接口和配置 schema。

Settings 以 AI 提供商和 Agent 运行时为主。提供商编辑器左侧选择“连接与认证”或某个模型，右侧显示当前内容；可以修改枚举协议、服务地址及 API key，模型 ID、名称／图标和可选能力在该模型内容中直接编辑。模型可手动添加或由模板快填，模板管理是提供商页上下文入口，没有先建全局模型／认证资源／模型路由的步骤。单目标调用直接选择提供商下的模型。协议首先支持原生 ChatCompletions v1 与 Responses v1，不翻译协议；fail-over 仍禁用。

双击进入编辑是全局交互范式，适用于设置中的可编辑列表和会话标题；使用系统 List 主操作，单击选择与平台多选保留。双击会话仍写入运行时的原生标题，不建立本地覆盖。

Agent 运行时可配置多个实例，每个实例选择精确版本类型及独立配置目录；工作目录属于具体会话。当前提供 Pi 1.0.2、Codex 0.159.3 和 DeepSeek Harness 0.2.0-rc.2，保持一个活跃执行 runner，空闲时切换会话；读取历史不进入执行。Pi 在准备边界要求正值上下文窗口；描述性能力未知不构成所有运行时的全局拒绝条件。添加实例后即可读取会话列表和详情，不需要连接或模型。新建表单选择实例、目录与模型后自动准备；历史模型选择与发送按会话进行。Pi 恢复可验证的原生选择记录，Codex／DSH 继续前明确选择，不按同名推断提供商。发送时自动准备／恢复，可观察工具结果、回答原生交互与取消。用户消息为右侧气泡，LLM 与工具左侧左对齐，只有系统／Harness 通知居中，不显示作者头像或昵称。历史由各 Harness 持有，Mac 只投影。

Harness 仅收到 Velune 本机网关配置，提供商 key 不传入执行 Harness。application 管理提供商私有认证，gateway 只接收中立异步解析接口，AI provider 使用当前解析出的短生命周期认证。API key 可查看编辑，OAuth 状态与登录放在原提供商上下文，运行时名称仅作为来源说明。OAuth adapter 保留原 SDK 的登录与刷新，不复制 refresh credential。修改来源绑定的目标时明确更换认证或清除，不能继续沿用旧目标授权。更换认证或模型执行配置后，受影响执行准备失效，历史视图保留，下一次发送重新准备；真实登录与调用由用户验收。

提供商字段与 API key 一次原子保存，认证编辑明确区分保留、设置新 key 和清除；读取失败或取消草稿不会清空旧值。不建立第二份凭据文件或 Keychain 补偿事务。配置只接受 schema 7，旧 schema 普通配置打开时原子重置为空当前配置，不迁移、不保留旧文件或备份；原 Harness、会话及已有平台秘密不删除。未来 schema 或损坏 JSON 明确报错。模板只是填写快照，不包含认证、服务地址或 Pi 投影，模板更新／删除不改变已有模型。

AI 提供商页的“从运行时导入…”是完整配置导入入口。选择已配置的 Agent 运行时实例后读取预览，复用该实例的目录与 Node 配置，再选择提供商模型导入；模型能力归导入的提供商条目。原提供商与认证文件保留；导入应用时集中保存静态 API key 到提供商私有配置，OAuth 保留来源刷新。重复项默认跳过，明确替换才更新已导入提供商；当前会话模型不自动改变。运行时实例、来源或目标配置在预览后变化时，须重新读取。读取配置不要求执行准备，也不要求当前会话已有模型或路由。仅支持当前网关能够保留语义的配置；不支持项及原因在预览中显示，动态命令不执行。

schema 7 采用 hard-cutoff，旧普通配置打开时直接重置，之后重新配置运行时与提供商；原 Harness 会话和认证保留。活动网关发生导入变更后使执行准备失效，重复跳过保持现有执行状态。提供商编辑器证据见 [AI 网关任务](../tasks/ai-gateway-audit/packet.md)，本轮多运行时构建、隔离验证与最终安装状态见 [多运行时任务](../tasks/multi-runtime/packet.md)。

隔离视觉预览使用 `--preview`（合成多轮会话）或 `--preview-empty`；Apple app 不设置独立深色验收入口。预览 Store 无 Transport，不打开 Application、不访问真实配置、Keychain 或会话、不调用模型。dyld 加载惰性的库文件不等于打开运行时，预览不能证明真实循环完成。

## 会话内容的隔离复验

内部角色、类型内容和 Unix 毫秒通过生成绑定消费；Mac 的稳定展示行不是持久化记录。标题在列表、只读打开和准备执行间保持同一含义，来源文件名不作为显示标题。`scripts/manual-session-browser.py` 使用固定 Pi SDK 在临时 HOME 创建带标题、无标题、正文、推理和工具调用／结果的合成历史，验证可读标题、类型角色和时间，以及已完成工具输出；同时保留无模型浏览与准备失败保留视图的验收。必须传入匹配的 bundle、Python bindings、Node 与外部 Pi CLI 绝对路径，不读取既有会话。

Markdown 渲染和长列表的具体手动操作／临时脚本、固定依赖、性能观察及构建安装版本归 [展示任务](../tasks/conversation-presentation/packet.md)。GUI 复核需要检查标题／时间、首次打开到底部、上翻时不抢滚动、返回底部、用户气泡、工具与正文左对齐、完整 Markdown 与长历史滚动。编译或模型层脚本成功不构成这些视觉体验的验收。

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

## 历史模拟原型的实现范围

- SQLite：任务树、逻辑会话、不可变模拟 Segment、重启 Run、Attempt 的资源／账户／协议、消息状态与事件。事务提交注入意图后才调用适配器，结果与 outbox 同事务落库。
- 路由：明确资源 allowlist、协议匹配、模拟与资格过滤、账户粘性，输出 dispatch／wait／handoff_required／needs_decision；不把调度 Harness 当 LLM 路由。当前同协议候选按输入列表优先级决定。
- 协作：同 scope 发现与一层委派、幂等冲突检测、到期阻断、结果回传／消费和独立 fixture 验收。只有可信宿主 API，没有网络身份校验；不准把 caller ID 参数直接暴露给不可信模型。
- 故障：busy／确认未发送可在截止时间内由调用方重试；提交结果 unknown 不重放并阻塞同 Segment。重启保留队列、结果与任务；进程死亡不会凭空释放未知状态。无原生核对能力时保持 blocked，不能继续跑真实副作用。
- mock provider 直接产生确定性文本，Protocol 枚举只是路由兼容标签，未编解码 Responses／Messages 流。三 Native 边界均 `native_verified:false`，NativeUnavailable 显式报 unsupported。

未实现：真实模型/provider 调用验证、网络 gateway、MCP server、ACP、完整账户权益／费用预留／用量、通用权限／审批、运行中真实工具取消、自动 handoff、任意任务调度、数据库迁移、FFI／分发安装器。Pi RPC 与 SDK 会话列表已有隔离实现，但仍只按合成子进程验收。当前 Mac bundle 的构建与隔离验证结果见 [首循环任务](../tasks/pi-mac-first-loop/packet.md)。无真实登录、秘密读取、付费调用。模拟“费用为零”不推定真实未知用量为零。

## 历史原生协议参考

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


## 原生运行时与历史读取

运行时配置引用精确 versioned variant，family 只负责分组；不同 breaking-change 版本通过独立 variant 与 adapter 支持，不把版本校验退化为接受新版本的提示。启动时从所选可执行文件的 `--version` 输出提取唯一版本，再按该 variant 的 regex 校验；协议握手另检查能力。

| variant ID | family | 版本 regex | 控制与历史 |
| --- | --- | --- | --- |
| `pi-1.0.2` | `pi` | `^1\.0\.2$` | Pi RPC、SessionManager SDK 分支历史 |
| `codex-0.159.3` | `codex` | `^0\.159\.3$` | Codex app-server thread／turn 与原生分页历史 |
| `dsh-acp-0.2.0-rc.2` | `deepseek-harness` | `^0\.2\.0-rc\.2$` | DSH ACP v1 session 控制、huihua DeepSeek provider |

Node 本体不随 Mac bundle，当前各实例配置 Node 22.19+ 的绝对路径；Codex 历史直接调用 app-server 原生 thread API。Codex／DSH 可执行文件由用户安装，实例保存绝对路径，不依赖 Finder 的 PATH。Codex 启动 app-server；DSH 当前版本通过配置的 Node 执行其 CLI JavaScript 入口并传 `--profile acp --patch <实例投影文件>`。CLI 版本不是 ACP 握手中的插件版本。

[`install-runtime-support.sh`](../scripts/install-runtime-support.sh) 按 [`package-lock.json`](../packages/agent-runtime/runtime-support/package-lock.json) 安装 huihua `0.2.0` 及其依赖（共五个 npm 包）到忽略的 `target/runtime-support`，禁用 npm lifecycle scripts，不修改全局安装。Mac 构建只分发其 parser 依赖和薄 adapter helpers，随包包含完整许可声明、licenses 和依赖锁；清单见 [第三方声明](../packages/agent-runtime/runtime-support/THIRD_PARTY_NOTICES.md)。不打包整个 DSH runtime 或 Codex CLI，也不复制 huihua parser 源码。

Codex 历史使用短生命周期 app-server 的 thread/list、thread/read 与 thread/items/list，不准备模型或网关。列表显式包含该版本全部来源与全部提供商，分页详情采用升序原生 items，Codex 负责 legacy／paginated 和 revert 历史继承。thread ID 不等于 rollout 文件 ID；不能扫描文件后要求 thread ID 唯一，也不按路径或时间挑选。huihua 桥仅供 DSH，通过公开 package exports 读取显式 home／roots，返回原生 ID、cwd、列表摘要与消息投影，不输出未知 payload。历史浏览不能代替 Codex thread/resume 或 DSH session/resume。 安全日志的 history_native_resolution 记录固定运行时家族、legacy／paginated 分类与读取／投影条数；history_native_failed 记录固定阶段和错误分类，不记录会话 ID、路径或正文。

Codex 历史人工验收入口为 [`manual-codex-history-identity.py`](../scripts/manual-codex-history-identity.py)。传入绝对路径 `--codex`、`--node`、`--bindings`、`--library` 与 `--resources`，加 `--exercise-revert --probe-items --probe-many-items`。脚本在临时 HOME／CODEX_HOME 中通过实际原生 fork、revert、exec 与 loopback Responses 验证列表身份、保留前缀、重启和多页顺序；不读取用户真实历史。可选 `--old-helper` 只用于显式旧版诊断对照，不能传当前 DSH-only helper。

原生历史只读浏览不建立网关或执行进程；Codex／DSH 缺少提供商身份的模型记录不自动匹配，继续前明确选择。DSH 切换模型先关闭会话，重启注入目录，再恢复同一原生 ID 并选择 upstream 公布的模型选项；不构造替代 opaque option。reasoningEfforts 需要实际 wire 映射，当前能力等级列表不足以证明映射，保持 SDK 默认，不推测支持。

审批与问题回答是类型化 pending interaction，用户选择、回答或取消后由 adapter 编码原生回复；不自动批准。取消执行与取消一个交互不同，执行终态按各协议观察。DSH 只暴露 ACP committed semantic 更新，不宣称原始 provider token delta、DSH 专有展示或历史 replay。提供商导入／订阅登录仍限 Pi 来源，新增执行 adapter 不等于新增导入能力。

DSH 的 Velune 执行 overlay 禁用 settings、llm-deepseek 和 llm-deepseek-account，使执行模型目录与 ACP 选择只指向网关；原 provider 配置不删除。上游 CLI 准备 ACP profile 的行为仍存在，不能声称原目录完全无写入。真实来源、认证与会话禁止用于开发验证；隔离循环只使用临时目录、合成输入与 loopback 上游。当前安装与用户验收状态由 [任务 packet](../tasks/multi-runtime/packet.md) 记录。

## Pi 运行时与本机网关

Rust agent-runtime 的 [Pi adapter](../packages/agent-runtime/src/client.rs) 处理显式 Node／CLI 路径与 JSONL RPC，同进程 application unit 负责运行时实例装配。会话列表使用固定 Pi 1.0.2 的 SDK helper，身份包含运行时实例，避免不同实例的同名会话混淆。RPC 没有 `list_sessions`；prompt 响应只表示接收，稳定终态为 `agent_settled`。取消先清队列再 abort，仍等待稳定终态后才允许关闭核心。

运行时配置保留入口、配置／状态根目录和可选会话存储目录，不要求工作目录。启用实例即加入统一历史列表，不要求手动连接或初始模型；执行前才准备实例网关，新建时使用原生目录选择器指定项目目录，恢复时读取 Pi 保存的 cwd。列表覆盖该实例的多个项目，空闲切换重新启动 Pi child 并保留网关；已失效的目录明确报错，不能回退到应用启动目录。当前配置不包含 `workingDir`，旧 schema 不保留兼容字段。

依据为 Pi `v1.0.2` 固定提交 `cd32f7725fdbddbaecdff5b1e68491563394e0ca` 的 [RPC](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/rpc.md)、[模型配置](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/models.md)与 [SDK 列表例子](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/examples/sdk/11-sessions.ts)。用户所说的 Pi home 由适配器映射为本版本的 `PI_CODING_AGENT_DIR`，不假定存在 `PI_HOME` 上游变量。

Velune 不分发任何 Agent runtime、Pi SDK 或 CLI。Pi 实例必须配置外部安装的 CLI 与 Node 22.19+ 绝对路径；Mac 在空字段尝试发现，已有路径不覆盖。薄 `pi_sdk.mjs` 从所选 CLI 的真实路径核对对应包名、版本、公开入口和 bin 关联；执行、历史、导入与 OAuth 使用同一安装，不从应用资源或其它实例回退。OAuth 来源保存导入时的安装路径、版本类型与精确 SDK 版本，来源安装缺失或变化须明确重新绑定。`install-pi-runtime.sh` 仅供开发者在忽略的 `target/pi-runtime` 安装隔离外部 Pi 1.0.2，既不是构建前置条件，也不进入应用包。

“会话存储目录”覆盖 Pi 历史文件的默认存储位置，不是会话工作目录。通常留空，Pi 使用运行时目录下 `sessions/` 并按工作目录编码分组；恢复已有会话时保留其文件所在目录。Velune 只投影这份历史，不另建会话数据库。

Application 在 `VELUNE_HOME/runtime-projections/` 下按运行时实例生成网关模型目录与 selection 文件，只列出 Velune 网关端点、模型投影与临时本地访问凭据，不写上游端点或 Keychain 引用。原运行时目录仍是 Pi home，保留原 `models.json`、认证、设置与会话。固定 SDK RPC launcher 通过独立的 ModelRuntime 注入受管目录，不覆盖原配置。运行时指定的 Pi 入口必须对应固定 1.0.2 SDK，不能对应的 wrapper 或其他版本明确拒绝，不静默替换安装。恢复历史会话后重新绑定 Velune 网关模型，历史 provider 不能绕过网关。每次实际请求由 application 的认证解析器校验提供商目标；API key 从捕获的私有配置解析，OAuth 调用内部来源 adapter。来源 adapter 使用原存储锁刷新，不复制 refresh credential；无 Mac Keychain shim。Unix helper 的超时、取消和关闭终止整个进程组；Windows helper 暂不开放。开发验证不读取真实 Keychain 或来源文件。认证 helper 的 AuthStorage 文件入口绑定固定 Pi 1.0.2；版本不符明确拒绝，是依赖升级时须复核的边界。

执行 Pi 默认选择 `velune/auto`。Core 为本轮决定具体逻辑模型，virtual model 返回相同模型的能力；Pi 的物理模型 ID 使用非秘密路由绑定的稳定身份，网关将该 ID 直接映射到同一次已配置路由，不再二次选择；逻辑模型 ID 保留在界面与选择状态中。Pi 的分支 state 保存实际选择，assistant 历史记录实际模型；新会话在创建表单选择模型，运行时不保存默认模型。模型 `maxTokens` 用作 Harness 元数据；输出参数仅按提供商协议发送和校验，订阅来源不支持服务端输出硬上限，不为其注入 `max_output_tokens`。Pi 的标准推理等级通过 `thinkingLevelMap` 限制为模型声明的等级；本轮不替自定义服务等级猜测转换规则。ChatCompletions 网关保留原生消息、推理历史、多模态内容与未知扩展；实际输入能力仍由所选模型和 Pi SDK 决定。ChatCompletions 与 Responses 保留原生请求 JSON、协议响应状态、安全请求／响应头及 JSON／SSE 内容；Responses 包括工具与 encrypted reasoning，只支持 foreground 创建，不增加查询、删除或 background API。客户端断开或网关停止取消活跃派发；同协议网关不合成 `[DONE]` 或业务终态；异协议按已确认的上游终态映射目标事件，不从 EOF 补造成功。网关不自动重试或切换提供商。合法的 Responses incomplete／failed 等业务终态原样返回，不将其误判为传输断流。

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

先构建动态库，并按 [bindings unit](../packages/bindings/README.md) 生成与其匹配的 Python 绑定。提供商配置验收使用 [提供商与模板脚本](../scripts/manual-provider-configuration.py)，传入绝对路径 `--bundle` 和 `--bindings`，验证本机文件 API key 显式读取／编辑／重开、公开描述与日志不含 key、模型 ID／协议／地址编辑、模板快照和 schema 7 hard-cutoff。使用临时 HOME 与合成 key，不读取真实资料。

[安装包首循环脚本](../scripts/manual-pi-native-loop.py) 另传 `--node` 和外部 `--pi`，从配置运行时和导入开始，通过实际固定 Pi SDK 完成工具续写、下一轮消息、提供商 key／model ID／地址编辑后实际派发检查。上游为回环合成服务，不涉及真实 Keychain、模型服务或会话。原生 [HTTP 脚本](../scripts/manual-gateway-native.py) 检查 ChatCompletions／Responses JSON、SSE、状态及安全头的保真与取消。所有入口均为显式人工验收，不接入 CI 或自动化测试；实际 GUI／真实提供商由用户验收。

[多运行时脚本](../scripts/manual-multi-runtime.py) 使用实际 Codex app-server／DeepSeek ACP 与回环合成上游，检查创建、发送、原生历史投影、恢复、同 API ID 跨提供商选择及版本拒绝保留活动连接。除 `--bundle`／`--bindings` 外，传入绝对路径 `--node`、`--codex`、`--dsh`（DSH 包的 `lib/bin.js`）。所有 HOME、运行时目录与工作目录均为临时目录。

[交互脚本](../scripts/manual-runtime-interactions.py) 另传 `--node`，以合成 app-server 验证 UniFFI 到控制协议的审批、私密回答、拒绝非法／陈旧回复、取消与意外 EOF。它不替代 Mac GUI 验收，也不连接真实上游。

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

无默认 feature 的依赖树不含 agent-runtime；当前 application 配置校验仍通过 gateway，因而依赖树保留 gateway／AI-provider，不启动其执行能力。本地会话操作明确返回 Unsupported。该构建只证明运行时裁剪，不表示已支持远端操作或已完成 AI 执行依赖裁剪。Kotlin 生成物依赖 JNA 及 kotlinx-coroutines；生成成功之外还需编译检查。C# 工具为第三方，UniFFI 版本兼容性仍需后续 C# app 接入时验证。

## 会话驱动准备的人工复验

[历史浏览脚本](../scripts/manual-session-browser.py) 使用外部固定 Pi SDK 生成临时历史，传入绝对 `--bundle`、`--bindings`、`--node`、`--pi`。它核对没有提供商或工作目录已消失时仍能列出和阅读、选择模型不启动执行、准备失败保留内容与选择、外部路径拒绝和单实例读取失败隔离。脚本不读取真实会话，不请求真实模型，不加入 CI。

模型选择属于当前会话：新建必须选择模型，历史只有 Pi 保存的明确内部引用可恢复；Codex／DSH 无法证明提供商时要求选择。只读历史视图中尚未执行的选择暂存在内存，视图关闭前不写回 Harness；发送准备后由原生适配器处理模型与持久化。修改配置只使执行装配失效，不清空历史或要求手动连接。

顶部运行时／模型选择属于下一轮意图，浏览来源和当前执行上下文不覆盖它。`send_turn(runtime_id, model_record_key, text)` 在同来源恢复原生会话；目标不同则创建新原生会话并交接明确引用的历史文本。回切也创建新原生段，工具执行、审批、签名推理状态不重放。首版最多携带256 KiB交接文本，不静默截断，也不把字节上限当成模型的 token 限额。

关联保存在 `VELUNE_HOME/conversation-links.json`（schema 1、Unix权限0600），独立于提供商配置；仅有有序原生引用、截止数量及内容摘要、交接定位，不存正文。逻辑会话沿用首段身份和原生标题，重启从各原生历史恢复。列表来源用于分组及打开，快照 `context_runtime_id` 用于当前查询／取消／回复；摘要管理能力由 application 结合所有关联原生段计算。旧段截止后外部追加不会进入逻辑历史，截止前改写明确报错。当前尾段不可读取时已校验来源仍可看，但不能继续发送或伪造恢复。

[跨 Harness 手工脚本](../scripts/manual-cross-harness.py) 接收绝对 `--bundle`、`--bindings`、`--node`、`--pi`、`--codex`、`--dsh`，使用实际Pi→Codex→DSH→Pi、临时HOME和loopback验证多次接续、单逻辑行、重开消息身份、无重复交接包及截止校验。[失败边界脚本](../scripts/manual-continuation-boundaries.py) 接收绝对 `--bundle`、`--bindings`、`--node`、`--pi`，核对超长／不兼容拒绝、关联原子提交失败保来源、尾段不可读禁止发送、普通配置重置不删关联以及重启不自动重发。生成绑定必须对应该bundle的库；这些入口只用于显式人工验收，不接入CI、不读真实账户或会话、不调用真实服务。

## 公开模型模板目录

在 AI 提供商的更多菜单打开“模型模板”，选择“从公开目录添加…”后显式读取 models.dev。候选保留来源提供商与实际模型 ID，选择后进入模板表单，保存为独立可编辑快照；拉取不会创建提供商、选择协议／端点或更新现有模型。仅来源明确声明的 effort values 可填写推理等级，不把 reasoning boolean 变成等级列表。

目录获取归 application，由 Mac 工作队列调用具名 UniFFI API；只请求固定公开 HTTPS 地址，禁用重试与重定向，最长 30 秒、最大 16 MiB。失败说明不包含响应正文或网络配置。拉取入口不使用提供商认证，领域 AI unit 与网关不依赖目录。缺失／零上下文和输出规格保持未知，用户应按实际提供商要求核对后编辑。目录来源声明和 MIT 许可随应用包保留，详情不伪称提供商官方能力保证。 人工复验使用 [公开目录脚本](../scripts/manual-public-model-catalog.py)，传入绝对 `--bundle`、`--bindings`；只有显式 `--public-get` 才访问公开地址。脚本使用临时 HOME 验证拉取不写配置、模板保存／编辑／重开和提供商数量不变，并以合成解析数据核对来源 ID、未知规格、effort null 和输入边界，不接入 CI。

## 本地诊断

Mac 使用 `OSLog.Logger`，subsystem 为 `local.velune`；系统 Console 可按该 subsystem 筛选。Rust 的 `tracing` subscriber 由 bindings 为每个应用对象独立装配，JSON Lines 日志位于 `VELUNE_HOME/logs/velune.<日期>.jsonl`，按 UTC 日期轮转并保留最近 7 个文件。该策略限制文件数量，不提供总字节硬上限。日志目录在 Unix 平台使用 0700 权限。

失败信息包含诊断编号；以该编号查找日志中的 `operation_id`，可定位操作、错误代码、阶段、耗时与底层原因。Swift 失败日志使用同一编号，其本地操作编号另行命名，不代表 Rust trace ID。常规 snapshot 和认证轮询不产生成功日志。诊断保存实际错误链、helper stderr 与失败上下文，不以隐私为由替换为固定说明；默认不采集所有请求、配置或会话正文，也没有远端导出。开发方仍只使用合成数据验证，不读取用户真实配置和秘密。

历史读取失败记录具体运行时实例的标识、名称、类型、实际配置的非秘密执行入口（binary／nodeBinary）与来源目录，以及阶段和原始原因。安装解析失败同时说明配置入口、已解析的 launcher 目标及 filesystem cause；实例槽位不能代替稳定标识。helper 启动、超时、退出、输出上限、来源读取、会话不存在与重复原生身份分别保留；子进程返回的实际错误文本应继续传到本地问题详情。旧错误若已在源头丢失原因，不能事后还原；新错误通过诊断编号关联操作和阶段。

网关请求另有 `gateway_request` span 的 `request_id`，不沿用建立执行准备时的 application `operation_id`。`gateway_request_received`、`gateway_route_selected`、`gateway_response_ready` 和 `gateway_request_finished` 描述入口与转发；子 span `gateway_attempt` 的 started／upstream_headers／finished 描述单次派发。目标序号只定位当前运行配置快照，不是跨准备的身份。结束字段区分传输完成、上游失败、调用方断开和网关关闭；完整转发失败 HTTP 响应仍可以是 request 的传输完成。准备响应不证明 TCP 客户端已收到，传输完成也不证明 LLM 业务成功。

[原生 HTTP 人工脚本](../scripts/manual-gateway-native.py) 使用临时 Rust probe 和合成回环服务，核对 JSON／SSE、HTTP 429、failed／incomplete 保真，以及头前取消、流中取消、在途关闭的关联与日志归因。它复用现有 tracing 依赖，不新增产品依赖或自动测试入口；[多运行时脚本](../scripts/manual-multi-runtime.py) 同时核对实际 bindings 日志的关联与秘密／配置排除。

领域包只发事件，不创建 subscriber。跨线程显式传播 dispatcher 和 span；将来增加 OTLP exporter 时在 bindings 的 layer 装配点扩展，当前没有远端导出、上传功能或 OTLP 配置。采用的库和契约见 [tracing dispatcher](https://docs.rs/tracing/latest/tracing/dispatcher/)、[tracing-appender 保留策略](https://docs.rs/tracing-appender/latest/tracing_appender/rolling/struct.Builder.html) 和 [Apple 日志指导](https://developer.apple.com/documentation/os/generating-log-messages-from-your-code)。

Mac 的“问题”窗口集中显示操作失败与当前读取问题，主界面工具栏提供入口，编辑／导入 sheet 在标题行提供入口；设置窗口保留系统标签导航，通过菜单“问题…”与 ⌘⇧M 打开问题窗口。新错误更新数量，不弹窗抢焦点；选择一项可查看并复制已有诊断字段。手动操作记录在当前进程内保留，成功操作不清空，用户可清除；持续会话列表或轮询问题按来源合并，恢复后移除，清除后若仍失败会再次出现。问题记录不是持久日志，不主动读取日志文件；收到的错误详情完整保留。界面不再在会话列表、composer 或设置底部重复展示这些错误，字段校验和业务进度仍在相应位置。

人工复验入口为 [`manual-problems.py`](../scripts/manual-problems.py)，传入绝对路径 `--bundle`、`--swift-build`、`--node`、`--pi`。它编译实际 Mac Store／Transport，使用临时 HOME 和真实 UniFFI，检查结构化失败的保留与消解；读取失败通过隔离 helper 控制，不读取用户原生会话。脚本不接入 CI，UI体验仍由用户验收。

## 会话管理

会话侧栏的上下文菜单提供原生重命名与删除；删除有永久删除确认。能力由所选版本化运行时描述，Pi 1.0.2 和 Codex 0.159.3 已接入，DSH 当前 ACP adapter 尚未接入。管理不要求模型、AI 认证或网关准备，但执行中不可操作。名称与删除结果写运行时来源并重新读取，不维护 Velune 本地覆盖。

会话加载时立即选中目标、详情显示进度；加载期间仅暂停其它行选择，菜单和重命名／删除提交可用。管理请求等待当前读取完成后按顺序执行并反馈状态，失败恢复明确的会话与内容关系。已加载与待加载身份独立，旧轮询按 generation 丢弃。实施与验收入口见 [任务](../tasks/session-management/packet.md)。

隔离手工入口为 `scripts/manual-session-loading.py`（`--bundle`、`--node`、`--pi`，需要同源码 SwiftPM 构建产物，可用 `--swift-build` 指定），以及 `manual-session-browser.py`、`manual-pi-native-loop.py` 和 Codex 原生历史脚本。AppStore 脚本以延迟 helper 驱动真实 UniFFI 读取，验证旧轮询、加载成功／失败恢复、多选与原生标题操作。入口均不进入 CI，不读取既有会话或调用真实模型。旧 session-management 重复入口已删除。

## 跨运行时会话浏览

列表统一汇总全部启用实例；运行时设置提供启用开关，禁用保留配置与原生数据，重新启用恢复历史。侧栏“显示”菜单负责分组、筛选和创建／更新时间排序，详情工具栏表示下一轮的运行时和模型意图。筛选或分组不切换会话。未知时间不推测，项目以完整 CWD 分组。label／section 是用户提供的组织方式例子，本轮不接入自定义分类；未来应独立保存 Velune 组织记录。

加载中可打开菜单、输入重命名并提交或确认删除，管理请求等待当前读取完成，再执行原生操作。显示等待／执行状态，加载失败不会丢弃已接受的管理请求，删除加载目标后不被旧快照重新插入。实施与验收见 [统一浏览任务](../tasks/conversation-browser/packet.md)。

运行时设置的“快速导入…”只发现可执行文件、公开版本和目录存在性，预览后才保存实例；未支持版本保留在候选列表中但不能导入。Node 仍是运行时配置的必填项。发现采用平台显式传入的用户目录和允许的非秘密目录覆盖，不读取原来源认证；提供商与模型导入通过后续显式预览操作执行。

会话浏览偏好 `conversationBrowserGroupLimit` 在应用配置中保存，默认 20、必须大于零。每个显示分组独立“加载更多”；当前来源仍汇总会话元数据后排序和分组，这不是上游 cursor 分页。macOS 使用原生 ⌘／Shift 列表多选，批量删除逐项修改原运行时会话并报告局部失败。提供商和运行时支持双击编辑；模型添加分别提供新建和独立模板选择 sheet，模板可批量选取并继续添加。

本轮隔离人工检查可运行 `scripts/manual-runtime-discovery.py`（指定临时 bindings、库、资源、Node 和外部 Pi CLI），覆盖公开版本发现、未支持版本、显式保存、重复候选以及分组数量持久化；`scripts/manual-messages-native.py` 覆盖原生 Messages 直接消费及网关 HTTP／SSE；`scripts/manual-runtime-messages.py` 覆盖合成来源导入及实际 Pi／DSH 接入。脚本不加入自动测试或 CI，真实服务、会话和视觉体验仍由用户验收。

顶部运行时／模型是纯粹的下一轮 draft；当前快照来源和 core execution owner 独立。公开发送统一为 `send_turn(runtime_id, model_record_key, text)`，旧 `send`／`select_model` 接口已删除。选择本身不准备运行时，历史无模型 marker 也可在发送时指定模型并准备；跨来源发送采用目标原生会话与文本交接，关联文件只保存引用和切换位置；不同 Harness ID 不直接 resume。历史失败日志的 source／execution／next-turn 配置序号分别命名，0表示当前没有实例。

外部 Pi 的安装发现与当前验证范围见 [外部运行时任务](../tasks/external-runtime-and-editing/packet.md)。已知 npm/pnpm literal launcher 只解析字面 CLI，不执行 shell 或继承其环境；无法解析的 wrapper 应手动选择安装包 CLI。公开版本执行失败与已识别但无 adapter 的版本分别显示，不能将坏安装误作不支持版本。

## LLM 协议转换的隔离验收

异协议采用 best-effort 请求／JSON／增量 SSE 转换，同协议继续原生透传。运行时 descriptor 中 `supportedProtocols` 表示它发给网关的协议，`supportedProviderProtocols` 表示通过网关可到达的提供商协议；两者不是同一能力。协议可到达不等于具体模型支持所有工具、输入模态或推理参数。

人工脚本 `scripts/manual-protocol-translation.py` 用临时 HOME、合成凭据与 loopback 上游驱动实际网关，不读取真实配置或调用真实提供商。它不是自动测试或 CI 入口。具体命令、已完成范围与外部 Pi／Codex 客户端证据归 [当前转换任务](../tasks/llm-protocol-translation/packet.md)。Messages 转换缺少请求输出上限时使用模型配置上限，不使用硬编码 token 数；无法映射的字段记录无正文静态诊断，真实流中断仍报告失败。


消息列表通过 Rust 的 typed turn 引用显示用户问题、工作过程与最新结果。中间消息默认折叠，展开状态按稳定 turn ID 保存；原生历史替换后清理已失效 ID。底部操作区提供用户消息 outline sheet 和向下箭头；选择 outline 条目后定位相应用户消息。设置 → 会话 → 消息列表可选择“用户消息大纲”：开始只显示用户消息，选中后保留此前用户索引，并从该位置展开后续完整内容；点击已选项返回大纲，切换会话或锚点被删除也返回大纲。显示偏好持久化到现有 `VELUNE_HOME` 配置，展开位置是当前视图状态，不写入 Harness。

运行时 stderr 使用有界本地缓存，并持续排空进程管道；达到容量后在诊断中明确标记截断，不能把容量限制伪装成隐私过滤或停止读取管道。工作时长是本机观察的执行区间，精度受轮询间隔影响；它不是提供商统计，也不回填到原生历史。

本次消息列表与诊断的隔离验收入口是 `scripts/manual-transcript-outline.py`、`scripts/manual-problems.py`、`scripts/manual-error-diagnostics.py`。这些按需手动脚本使用临时配置与合成内容，不接入自动测试或 CI；消息流脚本使用外部已安装 Pi 与本地服务，不依赖真实模型账户。

## 用量分析

“分析”使用独立原生窗口，从主窗口工具栏或显示菜单打开。记录来自启用后经过 Velune 的 LLM 网关请求，保存在应用配置根目录的 analytics.sqlite；不从诊断日志重建用量，不代表其它客户端或既往运行时调用。分析目录随 VELUNE_HOME 或平台传入的配置根目录确定，gateway 本身不读环境变量。窗口提供时间范围、已报告用量及覆盖率、每日趋势、提供商／模型拆分与请求详情；时间按平台本地日边界查询。

缓存输入已包含在输入总量中，推理输出已包含在输出总量中，详情中的子项不能再相加。有效输出速率包含上游等待，以相同有效请求的已报告输出 token 总量除以请求耗时总量；首输出延迟仅在流式语义输出可观察时存在。没有报告与报告零分开显示。具体测量与单位边界见 [AI 服务分析契约](design/ai-service.md#用量与性能分析契约)。实现与隔离人工验证结果见 [分析任务](../tasks/usage-analytics/packet.md)，此说明不代表真实提供商或 GUI 已验收。

人工验收入口为 `scripts/manual-gateway-analytics.py`（可用 `--fixture-out` 保存实际采集的合成 SQLite）、`scripts/manual-analytics-storage.py`（指定 `--bindings`、`--library`、`--resources`）和 `scripts/manual-analytics-ui.py`（指定实际合成 `--database`、`--library`、`--swift-build`）。它们分别贯通 HTTP→采集→持久化、实际 UniFFI 全量聚合与分页、实际 Mac Store 查询和展示状态，不接入 CI。请求明细最多展示500条，概览与趋势仍遍历该范围的全部记录。存储不可用或队列丢弃会明确显示告警，不用零计量掩盖缺失记录。
