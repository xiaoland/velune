# 开发与复验

项目不保留或新增自动化测试，以类型安全、静态检查和构建为先。行为验收使用手动操作或临时端到端脚本，不建立单元或集成测试套件，也不把临时脚本接入 CI。

内部不变量优先由类型表达：解析后的对象携带所需关联，枚举区分有效状态，穷尽匹配让新增变体触发编译错误。已在配置入口验证的不可变数据不在每次派发时全量重验。lint 用于有明确代码模式和实际维护收益的规则，不以扫描字符串或禁止所有条件分支代替类型设计。文件内容、用户配置、HTTP 与子进程响应仍在接入边界解析；运行时版本、目标存在性和实际执行失败不能预先由编译器证明。

## 当前 Mac 会话界面

当前应用为 SwiftUI 原生 Mac control surface：系统侧栏和工具栏、Chatbot 阅读与输入区、独立 Settings 窗口。系统决定基础字体、语义颜色及明暗外观，Velune 品牌仅保留在图标与少量细节。应用通过 UniFFI 0.32.2 生成的类型接口嵌入 Rust lib，消费 application 的具名用例，不解析 Harness 原生事件，也不启动常驻 Host 或访问 Unix socket。下文 IPC v1 与模拟体验段落描述历史原型，不能作为当前产品接入说明。

共享品牌 SVG 原稿与原始说明归属仓库级 [assets/brand](../assets/brand)，不归属任一平台 app。Mac 应用图标由 `scripts/render-app-icon.swift` 将该目录中的 graphite 光学稿生成十档传统 ICNS 资源。Dock／应用库使用的图标应保留 macOS 的透明光学留白，不能将品牌稿直接铺满画布；具体绘制范围在生成器中维护，不能通过修改品牌 SVG 补偿平台外框尺寸。

Mac 使用根目录 SwiftPM manifest 与 Package.resolved 锁定 swift-markdown 及其传递依赖 swift-cmark；构建脚本先生成UniFFI模块，再用SwiftPM的Xcode后端构建本机架构产品，固定 macOS 15 部署底线并随包保留 Swift 许可证。swift-markdown 0.9.0 的 manifest 需要 Swift 6.2 或以上工具链。应用与生成绑定目标各自启用warnings-as-errors，依赖保持其声明的Swift语言模式；不全局提升第三方警告。当前依赖没有资源bundle。不会构建或执行依赖的测试 targets。安装历史读取依赖后构建：

```sh
./scripts/install-runtime-support.sh
bash scripts/build-macos.sh
open /Applications/Velune.app
```

RelayKit 接入构建还需要 Go 工具链，桥接模块版本及依赖由 `packages/ai-provider/relaykit-bridge/go.mod` 与 `go.sum` 固定；Go 最低版本以该模块声明为准。Go 转换库作为同进程依赖链接，不启动独立网关后台。Mac bundle 的 ThirdParty/RelayKit 保留上游、Go runtime 和传递依赖许可证，以及模块锁文件；“开源许可…”入口包含 New API 署名与原项目链接。构建 manifest 记录 Go 版本和两个模块文件的摘要。具体平台与行为验收见 [RelayKit 任务](../tasks/new-api-reuse/packet.md)，本机成功构建不代表其它平台已通过验证。

应用根目录由启动进程的 `VELUNE_HOME` 环境变量指定，未设置或留空时为 `~/.velune`，非空值必须是绝对目录路径。平台将该路径显式传给 Application，库不自行读取环境。根目录下的 `generic-config.json` 持久化 AI 网关与 Agent 运行时实例配置，API key 由提供商私有认证配置保存，文件权限为 0600；订阅仍委托原 Harness 登录与刷新。公开列表不含 key，查看编辑通过提供商内显式操作。Finder 启动通常不继承 shell 配置，隔离开发可直接带环境运行 bundle 可执行文件：

```sh
VELUNE_HOME=/absolute/path/to/isolated-home /Applications/Velune.app/Contents/MacOS/Velune
```

构建脚本默认将验证签名后的 bundle 安装到 `/Applications/Velune.app`；仅构建可用 `bash scripts/build-macos.sh --build-only`。修改后退出应用再重建安装，安装器不会覆盖尚未退出的应用。用户已授权开发方随时直接退出 Velune；先正常退出，若应用拒绝退出，可终止已确认的 Velune 应用进程后安装，无需要求用户手动退出。产品显示版本来自仓库 `VERSION`，当前为 `0.1 beta.1`，原生“关于 Velune”面板读取同一版本。manifest 保存源码提交、UniFFI 版本、类型接口和配置 schema。

Settings 以 AI 提供商和 Agent 运行时为主。提供商编辑器左侧选择“连接与认证”或某个模型，右侧显示当前内容；可以修改枚举协议、服务地址及 API key，模型 ID、名称／图标和可选能力在该模型内容中直接编辑。模型可手动添加或由模板快填，模板管理是提供商页上下文入口，没有先建全局模型／认证资源／模型路由的步骤。单目标调用直接选择提供商下的模型。协议首先支持原生 ChatCompletions v1 与 Responses v1，不翻译协议；fail-over 仍禁用。

双击进入编辑是全局交互范式，适用于设置中的可编辑列表和会话标题；使用系统 List 主操作，单击选择与平台多选保留。双击会话仍写入运行时的原生标题，不建立本地覆盖。

Agent 运行时可配置多个实例，每个实例选择精确版本类型及独立配置目录；工作目录属于具体会话。Rust packages 当前提供 Pi 1.0.2、Codex 0.159.3、Claude Code 2.1.286 和 DeepSeek Harness 0.2.0-rc.2，保持一个活跃执行 runner，空闲时切换会话；读取历史不进入执行。Pi 模型能力可以未知，由 SDK 处理缺省；未知规格不阻止准备或原生请求。添加实例后即可读取会话列表和详情，不需要连接或模型。新建表单选择实例与模型，目录留空使用运行时默认；Mac首次发送时自动准备并创建原生会话；历史模型选择与发送按会话进行。Pi 恢复可验证的原生选择记录，Codex／DSH 继续前明确选择，不按同名推断提供商。发送时自动准备／恢复，可观察工具结果、回答原生交互与取消。用户消息为右侧气泡，LLM 与工具左侧左对齐，只有系统／Harness 通知居中，不显示作者头像或昵称。历史由各 Harness 持有，Mac 只投影。

Velune 向 Harness 注入本机网关提供商配置，Velune 管理的提供商 key 不随该注入传给 Harness；这不禁止 Harness 使用其原有提供商与认证。application 管理提供商私有认证，gateway 只接收中立异步解析接口，AI provider 使用当前解析出的短生命周期认证。API key 可查看编辑，OAuth 状态与登录放在原提供商上下文，运行时名称仅作为来源说明。OAuth adapter 保留原 SDK 的登录与刷新，不复制 refresh credential。修改来源绑定的目标时明确更换认证或清除，不能继续沿用旧目标授权。更换认证或模型执行配置后，受影响执行准备失效，历史视图保留，下一次发送重新准备；真实登录与调用由用户验收。

提供商字段与 API key 一次原子保存，认证编辑明确区分保留、设置新 key 和清除；读取失败或取消草稿不会清空旧值。不建立第二份凭据文件或 Keychain 补偿事务。配置只接受 schema 8，旧 schema 普通配置打开时原子重置为空当前配置，不迁移、不保留旧文件或备份；原 Harness、会话及已有平台秘密不删除。未来 schema 或损坏 JSON 明确报错。模板只是填写快照，不包含认证、服务地址或 Pi 投影，模板更新／删除不改变已有模型。

AI 提供商页的“从运行时导入…”是完整配置导入入口。选择已配置的 Agent 运行时实例后读取预览，复用该实例的目录与 Node 配置，再选择提供商模型导入；模型能力归导入的提供商条目。原提供商与认证文件保留；导入应用时集中保存静态 API key 到提供商私有配置，OAuth 保留来源刷新。重复项默认跳过，明确替换才更新已导入提供商；当前会话模型不自动改变。运行时实例、来源或目标配置在预览后变化时，须重新读取。读取配置不要求执行准备，也不要求当前会话已有模型或路由。仅支持当前网关能够保留语义的配置；不支持项及原因在预览中显示，动态命令不执行。

schema 8 采用 hard-cutoff，旧普通配置打开时直接重置，之后重新配置运行时与提供商；原 Harness 会话和认证保留。活动网关发生导入变更后使执行准备失效，重复跳过保持现有执行状态。提供商编辑器证据见 [AI 网关任务](../tasks/ai-gateway-audit/packet.md)，本轮多运行时构建、隔离验证与最终安装状态见 [多运行时任务](../tasks/multi-runtime/packet.md)。

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
| `claude-code-2.1.286` | `claude-code` | `^2\.1\.286$` | Claude stream-json 控制、原生 projects 历史 |

Node 本体不随 Mac bundle，Pi／Codex／DSH 当前配置记录需要 Node 22.19+ 的绝对路径，其中 Pi／DSH 实际执行 Node helper；Codex 历史直接调用 app-server 原生 thread API。Claude 历史使用 Python 3 执行 Rust 内嵌的只读 helper。各 Harness 可执行文件由用户安装，实例保存绝对路径，不依赖 Finder 的 PATH。Codex 启动 app-server；DSH 当前版本通过配置的 Node 执行其 CLI JavaScript 入口并在 managed turn 传 `--profile acp --patch <实例投影文件>`。CLI 版本不是 ACP 握手中的插件版本。

[`install-runtime-support.sh`](../scripts/install-runtime-support.sh) 按 [`package-lock.json`](../packages/agent-runtime/runtime-support/package-lock.json) 安装 huihua `0.2.0` 及其依赖（共五个 npm 包）到忽略的 `target/runtime-support`，禁用 npm lifecycle scripts，不修改全局安装。Mac 构建只分发其 parser 依赖和薄 adapter helpers，随包包含完整许可声明、licenses 和依赖锁；清单见 [第三方声明](../packages/agent-runtime/runtime-support/THIRD_PARTY_NOTICES.md)。不打包整个 DSH runtime 或 Codex CLI，也不复制 huihua parser 源码。

Codex 历史使用短生命周期 app-server 的 thread/list、thread/read 与 thread/items/list，不准备模型或网关。列表显式包含该版本全部来源与全部提供商，分页详情采用升序原生 items，Codex 负责 legacy／paginated 和 revert 历史继承。thread ID 不等于 rollout 文件 ID；不能扫描文件后要求 thread ID 唯一，也不按路径或时间挑选。huihua 桥仅供 DSH，通过公开 package exports 读取显式 home／roots，返回原生 ID、cwd、列表摘要与消息投影，不输出未知 payload。历史浏览不能代替 Codex thread/resume 或 DSH session/resume。 安全日志的 history_native_resolution 记录固定运行时家族、legacy／paginated 分类与读取／投影条数；history_native_failed 记录固定阶段和错误分类，不记录会话 ID、路径或正文。

Codex 历史人工验收入口为 [`manual-codex-history-identity.py`](../scripts/manual-codex-history-identity.py)。传入绝对路径 `--codex`、`--node`、`--bindings`、`--library` 与 `--resources`，加 `--exercise-revert --probe-items --probe-many-items`。脚本在临时 HOME／CODEX_HOME 中通过实际原生 fork、revert、exec 与 loopback Responses 验证列表身份、保留前缀、重启和多页顺序；不读取用户真实历史。可选 `--old-helper` 只用于显式旧版诊断对照，不能传当前 DSH-only helper。

原生历史只读浏览不建立网关或执行进程；Codex／DSH 缺少提供商身份的模型记录不自动匹配，继续前明确选择。DSH 切换模型先关闭会话，重启注入目录，再恢复同一原生 ID 并选择 upstream 公布的模型选项；不构造替代 opaque option。reasoningEfforts 需要实际 wire 映射，当前能力等级列表不足以证明映射，保持 SDK 默认，不推测支持。

审批与问题回答是类型化 pending interaction，用户选择、回答或取消后由 adapter 编码原生回复；不自动批准。取消执行与取消一个交互不同，执行终态按各协议观察。DSH 只暴露 ACP committed semantic 更新，不宣称原始 provider token delta、DSH 专有展示或历史 replay。提供商导入／订阅登录仍限 Pi 来源，新增执行 adapter 不等于新增导入能力。

DSH 的 Velune 执行 overlay 为 llm-pi-ai 追加网关目录，不改 ACP 原生默认选择，也不禁用 settings 或原生提供商。DSH 历史 header 记录最后使用的 provider／model，因此 app 私有 overlay 保留恢复历史所需的无凭据模型 metadata；native 轮次将该条目的地址设为 loopback 端口 0，先显式选择既有模型再发送。上游 CLI 准备 ACP profile 的行为仍存在，不能声称原目录完全无写入。真实来源、认证与会话禁止用于开发验证；隔离循环只使用临时目录、合成输入与 loopback 上游。当前安装与用户验收状态由 [任务 packet](../tasks/multi-runtime/packet.md) 记录。

## Pi 运行时与本机网关

Rust agent-runtime 的 [Pi adapter](../packages/agent-runtime/src/client.rs) 处理显式 Node／CLI 路径与 JSONL RPC，同进程 application unit 负责运行时实例装配。会话列表使用固定 Pi 1.0.2 的 SDK helper，身份包含运行时实例，避免不同实例的同名会话混淆；helper 只重读大小或修改时间变化的会话文件，打开列表中的会话不再重新列出全部历史。RPC 没有 `list_sessions`；prompt 响应只表示接收，稳定终态为 `agent_settled`。取消先清队列再 abort，仍等待稳定终态后才允许关闭核心。

运行时配置保留入口、配置／状态根目录和可选会话存储目录，不要求工作目录。启用实例即加入统一历史列表，不要求手动连接或初始模型；managed 执行前才准备实例网关；现有 Mac 新建时使用原生目录选择器指定项目目录，恢复时读取 Pi 保存的 cwd。列表覆盖该实例的多个项目，空闲切换重新准备 Pi child，managed turn 终态释放执行进程和网关；已失效的目录明确报错，不能回退到应用启动目录。当前配置不包含 `workingDir`，旧 schema 不保留兼容字段。

依据为 Pi `v1.0.2` 固定提交 `cd32f7725fdbddbaecdff5b1e68491563394e0ca` 的 [RPC](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/rpc.md)、[模型配置](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/models.md)与 [SDK 列表例子](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/examples/sdk/11-sessions.ts)。用户所说的 Pi home 由适配器映射为本版本的 `PI_CODING_AGENT_DIR`，不假定存在 `PI_HOME` 上游变量。

Velune 不分发任何 Agent runtime、Pi SDK 或 CLI。Pi 实例必须配置外部安装的 CLI 与 Node 22.19+ 绝对路径；Mac 在空字段尝试发现，已有路径不覆盖。薄 `pi_sdk.mjs` 从所选 CLI 的真实路径核对对应包名、版本、公开入口和 bin 关联；执行、历史、导入与 OAuth 使用同一安装，不从应用资源或其它实例回退。OAuth 来源保存导入时的安装路径、版本类型与精确 SDK 版本，来源安装缺失或变化须明确重新绑定。`install-pi-runtime.sh` 仅供开发者在忽略的 `target/pi-runtime` 安装隔离外部 Pi 1.0.2，既不是构建前置条件，也不进入应用包。

“会话存储目录”覆盖 Pi 历史文件的默认存储位置，不是会话工作目录。通常留空，Pi 使用运行时目录下 `sessions/` 并按工作目录编码分组；恢复已有会话时保留其文件所在目录。Velune 只投影这份历史，不另建会话数据库。

Application 在 `VELUNE_HOME/runtime-projections/` 下按运行时实例生成网关模型目录与 selection 文件，只列出 Velune 网关端点、模型投影与临时本地访问凭据，不写上游端点或 Keychain 引用。原运行时目录仍是 Pi home，保留原 `models.json`、认证、设置与会话。固定 SDK RPC launcher 使用原运行时的 ModelRuntime，并追加独立生成的网关提供商，不覆盖原配置。运行时指定的 Pi 入口必须对应固定 1.0.2 SDK，不能对应的 wrapper 或其他版本明确拒绝，不静默替换安装。从 Velune 发起新一轮时按显式 native／managed 选择设置模型，只有 managed 选择注入网关；运行时原有提供商、认证与扩展继续由原 SDK 使用，不增加只准使用网关的执行政策。每次实际请求由 application 的认证解析器校验提供商目标；API key 从捕获的私有配置解析，OAuth 调用内部来源 adapter。来源 adapter 使用原存储锁刷新，不复制 refresh credential；无 Mac Keychain shim。Unix helper 的超时、取消和关闭终止整个进程组；Windows helper 暂不开放。开发验证不读取真实 Keychain 或来源文件。认证 helper 的 AuthStorage 文件入口绑定固定 Pi 1.0.2；版本不符明确拒绝，是依赖升级时须复核的边界。

Pi 的 managed 执行使用 `velune/auto`；native 执行直接选择原生 provider／model。Core 为 managed 轮次决定具体逻辑模型，virtual model 返回相同模型的能力；Pi 的物理模型 ID 使用非秘密路由绑定的稳定身份，网关将该 ID 直接映射到同一次已配置路由，不再二次选择；逻辑模型 ID 保留在界面与选择状态中。Pi 的分支 state 保存实际选择，assistant 历史记录实际模型；新会话在创建表单选择模型，运行时不保存默认模型。模型 `maxTokens` 用作 Harness 元数据；输出参数仅按提供商协议发送和校验，订阅来源不支持服务端输出硬上限，不为其注入 `max_output_tokens`。Pi 的标准推理等级通过 `thinkingLevelMap` 限制为模型声明的等级；本轮不替自定义服务等级猜测转换规则。ChatCompletions 网关保留原生消息、推理历史、多模态内容与未知扩展；实际输入能力仍由所选模型和 Pi SDK 决定。ChatCompletions 与 Responses 保留原生请求 JSON、协议响应状态、安全请求／响应头及 JSON／SSE 内容；Responses 包括工具与 encrypted reasoning，只支持 foreground 创建，不增加查询、删除或 background API。客户端断开或网关停止取消活跃派发；同协议网关不合成 `[DONE]` 或业务终态；异协议按已确认的上游终态映射目标事件，不从 EOF 补造成功。网关不自动重试或切换提供商。合法的 Responses incomplete／failed 等业务终态原样返回，不将其误判为传输断流。

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

## RelayKit 的真实提供商采集与重放

[人工入口](../scripts/manual-relaykit-provider.py) 以真实 Pi CLI 的小型代码阅读任务经过 Rust gateway 和 RelayKit，核对工具调用、实际读取、匹配调用 ID 的结果续轮和正常终态。`collect` 会调用指定提供商，须有本轮真实调用授权；不沿用旧 MiniMax 示例的历史采集预算。`replay` 使用固定响应，不读取提供商凭据或发送外部模型请求。两者均非 CI 入口。

执行时显式传入 Node、Pi JavaScript CLI、rustc 的路径，以及 `target/` 下尚不存在的输出目录。采集还需要私有环境文件、变量前缀、允许的 HTTPS 主机、实际提供商／模型和入口／上游协议；Python 未配置系统信任根时，可用 `--ca-file` 指定已有可信 CA 文件，不能关闭 TLS 验证。运行 `--help` 查看参数。脚本仅在 HTTPS 边界注入凭据，不跟随重定向，不保存认证头；每次任务最多三次上游请求，提供商失败后阻断进一步真实调用。输出只固定本次合成代码任务的脱敏协议材料，不读取既有会话。

已固定 [Ark 的两次真实 SSE 响应](../fixtures/relaykit/ark-glm-5.3-flash-pi-tool-loop.json)。例如在仓库根目录执行以下命令，把占位路径替换为本机已有工具路径；`--pi` 指向所选 Pi 包的 JavaScript CLI，不能传 shell launcher。每次使用新的输出目录。

```sh
python3 scripts/manual-relaykit-provider.py replay \
  --fixture fixtures/relaykit/ark-glm-5.3-flash-pi-tool-loop.json \
  --output target/relaykit-replay-01 \
  --node /path/to/node --pi /path/to/pi/dist/cli.js --rustc /path/to/rustc
```

重放会启动实际 Pi 读取合成文件，并检查上游模型、对话、工具 schema、结果关联及输出上限与采集时一致。动态 system／developer 提示不纳入比较，固定样本也省略这些提示；保留原有 SSE 事件顺序与工具参数分片，不包含认证信息或本机路径。

真实调用证明所记录的入口与上游方向；离线协议投影只证明已有响应样本能被消费，不能证明另一上游接口接受相应请求。具体供应商身份、版本、覆盖和缺失条件归 [RelayKit 任务](../tasks/new-api-reuse/packet.md)。

## UniFFI 的临时端到端验收

先构建动态库，并按 [bindings unit](../packages/bindings/README.md) 生成与其匹配的 Python 绑定。提供商配置验收使用 [提供商与模板脚本](../scripts/manual-provider-configuration.py)，传入绝对路径 `--bundle` 和 `--bindings`，验证本机文件 API key 显式读取／编辑／重开、公开描述与日志不含 key、模型 ID／协议／地址编辑、模板快照和 schema 7 hard-cutoff。使用临时 HOME 与合成 key，不读取真实资料。

[安装包首循环脚本](../scripts/manual-pi-native-loop.py) 另传 `--node` 和外部 `--pi`，从配置运行时和导入开始，通过实际固定 Pi SDK 完成工具续写、下一轮消息、提供商 key／model ID／地址编辑后实际派发检查。上游为回环合成服务，不涉及真实 Keychain、模型服务或会话。原生 [HTTP 脚本](../scripts/manual-gateway-native.py) 检查 ChatCompletions／Responses JSON、SSE、状态及安全头的保真与取消。所有入口均为显式人工验收，不接入 CI 或自动化测试；实际 GUI／真实提供商由用户验收。

[多运行时脚本](../scripts/manual-multi-runtime.py) 使用实际 Codex app-server／DeepSeek ACP 与回环合成上游，检查创建、发送、原生历史投影、恢复、同 API ID 跨提供商选择及版本拒绝保留活动连接。除 `--bundle`／`--bindings` 外，传入绝对路径 `--node`、`--codex`、`--dsh`（DSH 包的 `lib/bin.js`）。所有 HOME、运行时目录与工作目录均为临时目录。

[交互脚本](../scripts/manual-runtime-interactions.py) 另传 `--node`，以合成 app-server 验证 UniFFI 到控制协议的审批、私密回答、拒绝非法／陈旧回复、取消与意外 EOF。它不替代 Mac GUI 验收，也不连接真实上游。

[并行会话脚本](../scripts/manual-parallel-sessions.py) 与 [实时会话脚本](../scripts/manual-live-sessions.py) 传入 `--library`、`--bindings`、`--bundle`、`--node` 与外部 Pi 1.0.2 的 `--pi`（包内 `dist/bundle/cli.js`），以临时 HOME、合成 Pi 会话和回环上游分别检查多会话同时执行与 turn 结果，以及实际 Pi CLI 作为另一客户端运行 turn 时的实时重读、运行识别、进程中途被杀后不再运行和自身执行的排除。[实际 Harness 实时脚本](../scripts/manual-live-native.py) 另传 `--codex` 与 `--claude`，在临时 managed HOME 中由实际 Codex 0.159.3／Claude Code 2.1.286 CLI 对回环合成上游执行外部 turn，检查按其存活记录（写入锁、会话登记）的运行识别、打开会话的实时重读与进程被杀后不再运行。[变更审阅脚本](../scripts/manual-working-changes.py) 使用同样参数，在临时 git 仓库中检查变更列表、差异行与路径拒绝。[推理强度脚本](../scripts/manual-reasoning-effort.py) 传入 `--library`、`--bindings`、`--bundle`、`--node`、`--codex`、`--claude` 与 `--pi`，在临时 managed HOME 中由实际三种 Harness 经网关与回环上游执行，检查可选强度、各运行时模型目录（原生模型带 Harness 自身强度、managed 模型随后且带提供商名；Pi 在临时 agentDir 写入合成 `models.json`）、范围外强度未发送即拒绝、上游实际收到的推理字段只作用于所选 turn，以及 Pi 原生模型经其自身提供商带所选强度派发。[附件脚本](../scripts/manual-attachments.py) 参数相同，检查图片经三种 Harness 自身图片输入原样到达上游、非图片文件以绝对路径列入正文、仅图片的 turn、已发送消息在实时快照与原生记录中显示为正文＋图片＋文件，以及缺失、相对路径、目录与内容不符的图片在发送前整条拒绝。

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

顶部运行时／模型选择属于下一轮意图，浏览来源和当前执行上下文不覆盖它。`send_turn_with_selection` 显式接收来源会话、目标实例和 native／managed 选择，在同来源恢复原生会话；目标不同则创建新原生会话并交接明确引用的历史文本。回切也创建新原生段，工具执行、审批、签名推理状态不重放。交接文本不设额外字节门禁，也不静默截断；实际输入接受能力由目标运行时及模型处理。

关联保存在 `VELUNE_HOME/conversation-links.json`（schema 1、Unix权限0600），独立于提供商配置；仅有有序原生引用、截止数量及内容摘要、交接定位，不存正文。逻辑会话沿用首段身份和原生标题，重启从各原生历史恢复。列表来源用于分组及打开，快照 `context_runtime_id` 用于当前查询／取消／回复；摘要管理能力由 application 结合所有关联原生段计算。旧段截止后外部追加不会进入逻辑历史，截止前改写明确报错。当前尾段不可读取时已校验来源仍可看，但不能继续发送或伪造恢复。

[跨 Harness 手工脚本](../scripts/manual-cross-harness.py) 接收绝对 `--bundle`、`--bindings`、`--node`、`--pi`、`--codex`、`--dsh`，使用实际Pi→Codex→DSH→Pi、临时HOME和loopback验证多次接续、单逻辑行、重开消息身份、无重复交接包及截止校验。[失败边界脚本](../scripts/manual-continuation-boundaries.py) 接收绝对 `--bundle`、`--bindings`、`--node`、`--pi`，核对超长／不兼容拒绝、关联原子提交失败保来源、尾段不可读禁止发送、普通配置重置不删关联以及重启不自动重发。生成绑定必须对应该bundle的库；这些入口只用于显式人工验收，不接入CI、不读真实账户或会话、不调用真实服务。

## 公开模型模板目录

在 AI 提供商的更多菜单打开“模型模板”，选择“从公开目录添加…”后显式读取 models.dev。候选保留来源提供商与实际模型 ID，选择后进入模板表单，保存为独立可编辑快照；拉取不会创建提供商、选择协议／端点或更新现有模型。仅来源明确声明的 effort values 可填写推理等级，不把 reasoning boolean 变成等级列表。

目录获取归 application，由 Mac 工作队列调用具名 UniFFI API；只请求固定公开 HTTPS 地址，禁用重试与重定向，最长 30 秒，不额外限制目录总字节数。失败说明不包含响应正文或网络配置。拉取入口不使用提供商认证，领域 AI unit 与网关不依赖目录。缺失／零上下文和输出规格保持未知，用户应按实际提供商要求核对后编辑。目录来源声明和 MIT 许可随应用包保留，详情不伪称提供商官方能力保证。 人工复验使用 [公开目录脚本](../scripts/manual-public-model-catalog.py)，传入绝对 `--bundle`、`--bindings`；只有显式 `--public-get` 才访问公开地址。脚本使用临时 HOME 验证拉取不写配置、模板保存／编辑／重开和提供商数量不变，并以合成解析数据核对来源 ID、未知规格、effort null 和输入边界，不接入 CI。

## 本地诊断

Mac 使用 `OSLog.Logger`，subsystem 为 `local.velune`；系统 Console 可按该 subsystem 筛选。Rust 的 `tracing` subscriber 由 bindings 为每个应用对象独立装配，JSON Lines 日志位于 `VELUNE_HOME/logs/velune.<日期>.jsonl`，按 UTC 日期轮转并保留最近 7 个文件。该策略限制文件数量，不提供总字节硬上限。日志目录在 Unix 平台使用 0700 权限。

失败信息包含诊断编号；以该编号查找日志中的 `operation_id`，可定位操作、错误代码、阶段、耗时与底层原因。Swift 失败日志使用同一编号，其本地操作编号另行命名，不代表 Rust trace ID。常规 snapshot 和认证轮询不产生成功日志。诊断保存实际错误链、helper stderr 与失败上下文，不以隐私为由替换为固定说明；默认不采集所有请求、配置或会话正文，也没有远端导出。开发方仍只使用合成数据验证，不读取用户真实配置和秘密。

Pi RPC 子进程持续排空 stderr，最多保留1 MiB用于失败诊断，超出时显式说明截断；正常运行不输出该缓存。启动、控制管道写入、退出和超时失败保留已取得的原始错误，退出时同时提供进程状态。旧版丢弃的 stderr 无法事后还原；仅有“child exited before responding”的旧记录不能用于判断真实退出原因，需要新版本中的重试记录。 Pi 首次状态握手的单次响应等待窗口为15秒，允许原生会话、扩展及项目资源加载；普通控制命令仍为3秒。超时诊断同时记录当时进程状态，不能将超时等同于进程退出。 来源扩展注册提供商、模型选择和实际派发由 Pi SDK 处理；Velune 不额外拦截注册、不检查派发目标或拒绝名称覆盖。网关作为新增提供商装配，不作为唯一允许的请求入口。

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

会话浏览偏好 `conversationBrowserGroupLimit` 在应用配置中保存，默认 20、必须大于零。每个显示分组独立“加载更多”；当前来源仍汇总会话元数据后排序和分组，这不是上游 cursor 分页。macOS 使用原生 ⌘／Shift 列表多选，批量删除逐项修改原运行时会话并报告局部失败。提供商和运行时支持双击编辑；模型添加分别提供新建和独立模板选择 sheet，模板可批量选取并继续添加。 分组、运行时／项目筛选及排序方向也保存在应用配置中；它们只影响列表投影，不改变下一轮执行实例或原生会话。搜索文字和每组额外加载数量不持久化。

本轮隔离人工检查可运行 `scripts/manual-runtime-discovery.py`（指定临时 bindings、库、资源、Node 和外部 Pi CLI），覆盖公开版本发现、未支持版本、显式保存、重复候选以及分组数量持久化；`scripts/manual-messages-native.py` 覆盖原生 Messages 直接消费及网关 HTTP／SSE；`scripts/manual-runtime-messages.py` 覆盖合成来源导入及实际 Pi／DSH 接入。脚本不加入自动测试或 CI，真实服务、会话和视觉体验仍由用户验收。 `scripts/manual-browser-preferences.py` 使用临时应用目录，贯通 Mac AppStore、Transport 与 UniFFI，检查快速连续修改、重开恢复和保存失败回滚；要求指定当前debug动态库与Swift构建目录。

顶部运行时／模型是纯粹的下一轮 draft；当前快照来源和 core execution owner 独立。公开发送统一为 `send_turn(runtime_id, model_record_key, text)`，旧 `send`／`select_model` 接口已删除。选择本身不准备运行时，历史无模型 marker 也可在发送时指定模型并准备；跨来源发送采用目标原生会话与文本交接，关联文件只保存引用和切换位置；不同 Harness ID 不直接 resume。历史失败日志的 source／execution／next-turn 配置序号分别命名，0表示当前没有实例。

外部 Pi 的安装发现与当前验证范围见 [外部运行时任务](../tasks/external-runtime-and-editing/packet.md)。已知 npm/pnpm literal launcher 只解析字面 CLI，不执行 shell 或继承其环境；无法解析的 wrapper 应手动选择安装包 CLI。公开版本执行失败与已识别但无 adapter 的版本分别显示，不能将坏安装误作不支持版本。

## LLM 协议转换的隔离验收

异协议采用 best-effort 请求／JSON／增量 SSE 转换，同协议继续原生透传。运行时 descriptor 中 `supportedProtocols` 表示它发给网关的协议，`supportedProviderProtocols` 表示通过网关可到达的提供商协议；两者不是同一能力。协议可到达不等于具体模型支持所有工具、输入模态或推理参数。

人工脚本 `scripts/manual-protocol-translation.py` 用临时 HOME、合成凭据与 loopback 上游驱动实际网关，不读取真实配置或调用真实提供商。它不是自动测试或 CI 入口。具体命令、已完成范围与外部 Pi／Codex 客户端证据归 [当前转换任务](../tasks/llm-protocol-translation/packet.md)。Messages 转换缺少请求输出上限时使用模型配置上限，不使用硬编码 token 数；无法映射的字段记录无正文静态诊断，真实流中断仍报告失败。


消息列表通过 Rust 的 typed turn 引用显示用户问题、工作过程与最新结果。中间消息默认折叠，展开状态按稳定 turn ID 保存；原生历史替换后清理已失效 ID。详情右侧工具栏打开用户消息 outline sheet，选择条目后定位相应用户消息；消息列表内保留回到底部的向下箭头。设置 → 会话 → 消息列表可选择“用户消息大纲”：开始只显示用户消息，选中后保留此前用户索引，并从该位置展开后续完整内容；点击已选项返回大纲，切换会话或锚点被删除也返回大纲。显示偏好持久化到现有 `VELUNE_HOME` 配置，展开位置是当前视图状态，不写入 Harness。

运行时 stderr 使用有界本地缓存，并持续排空进程管道；达到容量后在诊断中明确标记截断，不能把容量限制伪装成隐私过滤或停止读取管道。工作时长是本机观察的执行区间，精度受轮询间隔影响；它不是提供商统计，也不回填到原生历史。

本次消息列表与诊断的隔离验收入口是 `scripts/manual-transcript-outline.py`、`scripts/manual-problems.py`、`scripts/manual-error-diagnostics.py`。这些按需手动脚本使用临时配置与合成内容，不接入自动测试或 CI；消息流脚本使用外部已安装 Pi 与本地服务，不依赖真实模型账户。该脚本直接调用 composer 使用的实际 AppStore 发送入口，经 Transport／UniFFI／外部 Pi 到合成 HTTP 上游，覆盖关闭后重开原生会话再发送；检查精确用户正文、所选模型、回复投影及原生会话身份。它不替代实际界面按键或真实账户验收。多工作区切片另覆盖同一共享核心下的两个未发送草稿、不同原生会话的首次发送和身份绑定续接、另一工作区生成时只读历史、关闭Tab保留活跃任务，以及配置广播不覆盖合法下一轮选择。


消息尺寸与连续滚动可按需运行 `scripts/manual-transcript-scroll.py`，传入显式 `--swift-build` 与 `--library`。它把实际 transcript 源编译为临时探针，在探针窗口中载入合成长历史，检查快速上滑穿过未测量历史时阅读行不位移、显示 TextKit 高度与测量帧一致、可见行连续且文档不宽于视口，以及远跳、宽度变化、阅读时尾部增长、局部展开和回到底部跟随；可见行连续之外，任何未分配行的可见单元格也视为失败；`--snapshot` 会显示探针窗口并只截取该窗口，用于目视核对 Markdown 与展开重叠。临时目录退出即清理，不接入CI；它不代表物理滚轮或真实历史体验。

`scripts/manual-store-idle.py` 使用隔离配置和实际AppStore成功回调，检查未变化输入不向应用／工作区／消息投影重复发布，同时确认真实问题恢复和配置修改仍传播。它与尺寸脚本一样是按需手动入口，不接入CI，不读取真实配置或会话。

## 用量分析

“分析”使用独立原生窗口，从“显示 → 分析…”打开。记录来自启用后经过 Velune 的 LLM 网关请求，保存在应用配置根目录的 analytics.sqlite；不从诊断日志重建用量，不代表其它客户端或既往运行时调用。分析目录随 VELUNE_HOME 或平台传入的配置根目录确定，gateway 本身不读环境变量。窗口提供时间范围、已报告用量及覆盖率、每日趋势、提供商／模型拆分与请求详情；时间按平台本地日边界查询。

缓存输入已包含在输入总量中，推理输出已包含在输出总量中，详情中的子项不能再相加。有效输出速率包含上游等待，以相同有效请求的已报告输出 token 总量除以请求耗时总量；首输出延迟仅在流式语义输出可观察时存在。没有报告与报告零分开显示。具体测量与单位边界见 [AI 服务分析契约](design/ai-service.md#用量与性能分析契约)。实现与隔离人工验证结果见 [分析任务](../tasks/usage-analytics/packet.md)，此说明不代表真实提供商或 GUI 已验收。

人工验收入口为 `scripts/manual-gateway-analytics.py`（可用 `--fixture-out` 保存实际采集的合成 SQLite）、`scripts/manual-analytics-storage.py`（指定 `--bindings`、`--library`、`--resources`）和 `scripts/manual-analytics-ui.py`（指定实际合成 `--database`、`--library`、`--swift-build`）。它们分别贯通 HTTP→采集→持久化、实际 UniFFI 全量聚合与分页、实际 Mac Store 查询和展示状态，不接入 CI。请求明细最多展示500条，概览与趋势仍遍历该范围的全部记录。存储不可用或队列丢弃会明确显示告警，不用零计量掩盖缺失记录。

Mac 主窗口底部面板在输入正文下方提供一条紧凑操作行：运行时、模型、发送／停止。选择菜单直接列出选项和设置入口，不各占半个面板宽度。侧栏工具栏提供分组筛选，新建和大纲位于详情右侧工具栏，应用级分析通过显示菜单访问。问题窗口保留菜单及快捷键；主窗口仅在存在问题时显示状态入口。分析窗口采用概览、提供商与模型、请求三个页面，表格独立滚动，双击或明确详情动作打开只读sheet，不在选中行时挤出侧栏。刷新保留最后成功的范围与数据，并标明正在更新或读取失败，成功后一起替换；输入和输出分别展示报告覆盖率，配对总量在指标说明中保留。

## Rust 运行时的模型来源、SSH 与 HOME

2026-10-10 的切片只交付共享 Rust／UniFFI 能力，未改动、构建或安装 app。平台入口仍待实现；不能根据核心验收推断当前已安装的 Mac 应用已有新选择器或配置入口。用例及类型契约归 [application](../packages/application/README.md) 和 [bindings](../packages/bindings/README.md)。

通过 `runtime_models` 读取目标实例的原生模型及 managed 模型候选，使用 `create_conversation_with_selection`／`send_turn_with_selection` 提交明确来源。原生提供商由 Harness 自行解析，不需先导入网关。`next_turn_model_selection` 是下一轮意图；managed turn 结束后仍选择 managed，不代表临时网关覆盖仍在运行。既有 `create_conversation`／`send_turn` 继续是 managed model 的便捷入口。

新建实例可用 `upsert_runtime_with_execution` 一次保存运行时与执行配置。`configure_runtime_execution` 更新已有实例，`runtime_execution` 返回具名配置。Local／Ssh 与 Inherit／Existing／Managed HOME 分别选择。本机 managed HOME 由应用按实例稳定分配；SSH managed HOME 需提供远端绝对根目录。既有账户不会复制到新 HOME，用户须自行配置和登录。普通配置重置或删除运行时配置不删除这些原生数据。

SSH 依赖本机 OpenSSH，以及远端 POSIX 环境和 Python 3；各 Harness 需在远端安装匹配版本，Pi／Codex／DSH 的当前配置还需提供远端 Node 路径。目标使用已配置的 OpenSSH 主机名／alias、可选端口和明确 SSH 可执行文件；采用非交互认证与严格主机校验，因此应由用户预先完成认证和主机信任。连接不转发认证 agent，不静默接受未知主机。远端资源目录需独立于原生配置目录；首次使用投送薄 helper 与已打包 parser 依赖，不安装 Harness。Python helper 源码内嵌在 Rust 中，不要求额外复制 `.py` 文件到 app 资源目录。

managed 请求通过远端 loopback 反向转发访问本机网关，释放执行资源时关闭隧道。远端监听核对使用 Linux proc 网络表，或 macOS／BSD 的 `lsof`；无法证明仅 loopback 时返回明确错误。断连清理所属进程组，不自动重发不确定结果。历史、版本、模型发现及 cwd 检查都在执行机器完成。跨机器接续目前沿用来源 cwd，目标必须存在同一路径；尚无目录映射或重定位入口。

Pi 提供商导入／OAuth 来源操作仍限定默认本机 execution，SSH 或显式 HOME 配置返回不支持；原生 provider 的直接执行不受此限制。Claude、Codex、DSH 的 provider 导入与订阅登录没有因为增加运行控制而自动实现。

人工验收可构建 Rust 动态库并生成相同版本 Python bindings，再运行 [选择与 HOME 脚本](../scripts/manual-runtime-selection.py)，传入 `--library`、`--bindings`、`--resources` 的绝对路径。该脚本只使用临时 HOME 和合成控制 peer，覆盖原生／managed 模型、终态／失败／取消清理、跨实例接续、原子执行配置和来源文件不变。实际 Claude CLI、真实 OpenSSH 本机隔离连接及旧运行时回归的适用条件归 [本轮 packet](../tasks/runtime-provider-ssh/packet.md)，不把合成 peer 当成真实外部 SSH 主机或真实提供商验收。

## 通用 ACP Agent

通用入口为运行时类型 `acp-v1`。它连接用户已安装的原生 ACP 程序或社区 bridge；通过 `initialize` 协商协议，不调用任意程序的 `--version`，也不按 Agent 名称建立白名单。将可执行文件的目标机器绝对路径写入实例 `binary`，参数与显式环境通过 `AcpLaunch` 配置；参数直接作为 argv 传入，不经过 shell。登录与运行时安装仍由用户完成，Velune 不导入或复制既有认证。

Rust application 与 UniFFI 提供 `upsert_acp_runtime`，一次保存实例、`RuntimeExecution` 与 `AcpLaunch`。ACP 不要求 `nodeBinary`、`agentDir` 或远端 helper 资源目录；如果具体 bridge 使用 Node，可直接将 Node 作为 binary、bridge 脚本作为参数。SSH 仍需目标机器上的 Python 3 来承载已有执行传输，HOME 使用继承、现有路径或 Velune managed 选择。

使用 `acp_runtime_info` 查看 Agent 身份、认证方法描述与协商能力。`create_conversation_with_selection` 配合 `ModelSelection::RuntimeDefault` 可创建无模型选择器的会话；普通发送、权限答复、取消使用现有 conversation API。原生模型目录只来自活动会话的 `configOptions`，`runtime_models` 另总是先列 Agent 默认；需要完整的模式、布尔选项或多个选择器时使用 `acp_session_config`／`set_acp_session_config`，保留 Agent 的 config ID 与 opaque value。配置更新以 Agent 返回结果为准。

有 list 能力的 Agent 使用通用会话列表；只有 load 的 Agent 可通过 `open_acp_conversation(runtime_id, native_session_id, cwd)` 显式打开。直接使用 `agent-runtime::acp::Client` 时可分别调用 load 与 resume，后者恢复上下文但不回放历史；application 的历史浏览和持久跨 Harness 接续要求 load。无 load 的连接仍可连续多轮运行，但关闭后不保证重开。通用 ACP 不支持统一的 managed provider 注入，应使用该 Harness 的专用 adapter 获取已实现的网关能力。

本轮不提供 App 的 ACP 配置、会话选项或能力展示界面，也不构建或安装 App。无真实认证与模型调用的手动验收入口为 `scripts/manual-acp-adapter.py`，具体构建产物、参数、实际 Agent 与 SSH 证据见 [ACP 任务](../tasks/acp-adapter/packet.md)。自动化测试不在仓库验证流程内。

2026-10-10 补充验收了两个原生 ACP 入口，均使用同一通用 adapter，没有新增 Agent 专用分支。Devin CLI `3000.11.3` 的启动参数为 `acp`，隔离未认证状态下通过握手、创建、模式修改、live list 和空会话删除；未验证真实 prompt，未发送 prompt 的空会话跨进程 load 失败。OpenCode `1.18.35` 同样使用 `acp`，经过 UniFFI 在本机合成 provider 上通过模型选择、prompt、流式答复、shutdown 后列表与历史重开。这些是特定版本和条件下的观察，不是对所有 ACP 实现或真实账户的兼容承诺。详见 [Devin 证据](../tasks/acp-adapter/devin-evidence.md)与 [OpenCode 证据](../tasks/acp-adapter/opencode-evidence.md)。

## Harness 子 Agent 协作

Rust／UniFFI 的 `create_collaborative_conversation` 创建新的父会话，传入父运行时、工作目录、模型选择、`CollaborationTarget` 列表和 `CollaborationLimits`。每个 target 是宿主明确授权的执行 profile，绑定运行时实例、目标机器工作目录与模型；不能由父 Agent 在工具参数里增加或覆盖。随后仍通过现有发送接口驱动父会话。普通会话不会自动获得协作工具，现有 App 尚无此入口。

父 Harness 的临时 MCP 服务提供 `agents`、`spawn`、`send`、`status`、`wait`、`cancel` 和 `close`。先发现目标，再传 `target_id` 与任务文本创建子 Agent；后续操作使用返回的 `child_id`。实际返回对象的身份字段为 `id`，状态对象同时带 `revision`、`turn`、`state`、`result` 与错误信息。`wait` 使用已读 revision，单次最多等待 30 秒；等待超时不取消子任务。可并行执行多个子 Agent，也可给空闲子 Agent 续发；忙时不会隐式排队或插话。子 Agent 不获协作工具，本切片只提供一层委派。

`collaboration_children` 查看子状态，`child_snapshot` 取得原生投影和待审批项，`reply_child_interaction` 需要子任务 ID、当前 turn 与原生交互 ID。父 Agent 的 MCP 工具不能代替用户批准子任务权限。父取消会级联取消活动子 turn；父 turn 正常结束保留子会话，供后续 turn 使用。显式 `close_collaboration` 关闭协作范围；普通应用关闭仍在父或子忙碌时拒绝并保留执行状态，资源析构负责最终清理。配置修改要求先关闭协作范围，防止授权快照与配置更新混用。

协调器与 HTTP MCP 服务在 application 进程内；每个子执行器独立轮询事件，等待工具结果不需要 UI 轮询，也不会持有 Application 的全局锁。Pi、Codex、Claude、DSH 和通用 ACP 使用各自会话级 MCP 注入，不修改原持久配置。SSH 使用会话持有的 loopback 反向转发；仅支持 stdio MCP 的 ACP Agent 使用内嵌 Python helper，目标机器需可运行 Python 3。helper 不管理任务状态或自动重试，不是独立 Host。

轮次期限和取消在执行器边界检查；初始化或控制 RPC 尚在同步等待时，以 adapter 自身的有界超时为准，不能承诺立即中断。取消或期限在启动期间到达后，不再提交原 prompt。原生历史仍归 Harness；Velune 不保存 transcript 副本，不自动恢复重启前的协作 scope，也不把子 Agent 的报告视为已经验收。App 子任务关系、状态和审批界面待实现。隔离验收与适用边界见 [Harness 协作任务](../tasks/harness-interoperation/packet.md)。

手动闭环入口为 [manual-harness-interoperation.py](../scripts/manual-harness-interoperation.py)，通过参数传入本次构建的 UniFFI 库、生成的 Python 绑定与 OpenCode／DSH／Node；脚本在 macOS sandbox 中使用临时 HOME 和 loopback 合成 provider，断言无前台轮询时子结果仍返回父 Agent。具体版本和复现方式见 [互操作验收](../tasks/harness-interoperation/interoperation-evidence.md)。

## Agent runtime 原生资源管理

`runtime_resource_catalog(runtime_id, cwd, kind, scope)` 查询指定实例的资源位置与支持状态；`read_runtime_resource` 读取一项，`write_runtime_resource` 保存一项，`delete_runtime_resource` 删除文件或空目录。四类资源为 `UserInstructions`、`McpServers`、`Skills` 和 `NativeAgentDefinitions`。`User` 使用该实例的原生配置位置，`Project` 使用显式 cwd；路径属于运行时的执行机器，SSH 不会回退到本机同名目录。查询不要求运行时启用、建立会话或安装执行 helper。

先读取目录和条目状态；`Unsupported` 不是空集合，`Missing` 不是读取失败。读取返回的 document 含原生字节、执行位和条目 revision；编辑或删除传 `Match(revision)`，新建传 `Missing`。发生 `conflict` 时须重新读取再决定，不能以无条件保存覆盖外部编辑。管理错误经 UniFFI 保留 `runtime_management` 类别与具体 code。资源访问失败不会重置配置或创建一个空替代。

普通资源 key 是类别根内相对路径；skills 按完整目录树列出附件，并按单文件读取、创建、更新和删除，保存时默认保留已有执行位。MCP key 是服务器名称，document 是该服务器的原生 JSON 对象；显式详情可能含 env 或 header 秘密，列表和诊断不含这些值。更新只替换该服务条目，未编辑的服务与其他原生配置字段保持；不把各 Harness 的 MCP 格式统一转换，也不执行或安装配置中的命令。

当前适配覆盖 Codex／Claude 的四类、Pi 的 instruction／MCP／skills、DSH 的 instruction／skills。Pi 没有内置 sub-agent 定义；DSH 的插件式 MCP／sub-agent 配置和通用 ACP 的私有资源布局没有接入本管理契约，会明确说明不支持。该列表是管理适配范围，不等于对应 Harness 的全部运行能力，也不是组织策略、插件与祖先目录合并后的有效配置视图。

修改在运行中的 turn 或协作 scope 存在时拒绝；停用实例仍可编辑原生资源。成功保存不自动重启或重载运行时，具体生效时机由 Harness 决定。受管 HOME 仍保存原生配置，不形成 Velune 同步副本。Mac 以“资源库”窗口管理这些资源：资源在 Velune 中定义，启用到运行时后由 Velune 写入并保持同步，原生被改时由用户选择处理（可先查看差异），忙时延后的写入在运行时空闲后自动补写；手动验收脚本为 `scripts/manual-resource-library.py`，真实 Harness 加载为 `scripts/manual-resource-library-native.py`（sandbox、无模型请求）。实施、具体原生路径与隔离验收见 [运行时资源管理任务](../tasks/runtime-management/packet.md)。

**待实现的 App 编译接入**：下次为 Mac 重新生成绑定时，须在 `app/mac/Transport.swift` 的 `kindName` 穷尽匹配中处理新增的 `BindingFailureKind.runtimeManagement`。当前非 UI 切片保留该 App 源码不变，不能直接把生成的新绑定替换进 App 后宣称构建通过。

手动 API 验收入口为 [manual-runtime-management-api.py](../scripts/manual-runtime-management-api.py)，真实 Claude／Pi 加载入口为 [manual-runtime-management.py](../scripts/manual-runtime-management.py)。后者要求 macOS sandbox、匹配的 CLI 和隔离安装的 Pi，使用 loopback 合成模型响应；运行参数、Codex／DSH 加载与 WSL 文件边界证据见 [固定版本验收](../tasks/runtime-management/native-evidence.md)。
