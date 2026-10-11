# Agent runtime 资源管理

## 目标与边界

2026-10-11 用户要求把 Velune 的 agent-runtime 管理从会话、模型扩展到 user instruction、MCP、agent skills 和 sub-agents，允许能力或资源缺失，仍限定非 UI。本任务贯通 Rust agent-runtime、application、UniFFI 与必要 helper／手动验收；不改 App，不安装或发布 App，不 push。沿用本地自主提交授权。开发验收仅使用隔离合成文件、HOME 和模型替身，不读取真实秘密、认证或会话，不调用真实模型，不新增自动化测试。

期望结果是宿主能发现运行时支持的管理能力、列出与读取已有资源，并在支持的边界新增、修改或删除，清楚区分不支持、尚不存在和读取失败。sub-agent 定义配置与当前协作 scope 的活跃子会话分开。运行时原有内容继续有明确归属，不为四类配置建立未经用户要求的同步副本。

## 实施基线与判断

开始本任务时，`e05c604` 已交付会话级 MCP 协作注入与独立子执行器。现有运行时包括 Pi、Codex、Claude Code、DSH 和通用 ACP，支持本地／SSH、继承／指定／受管 HOME，但尚没有通用的原生 instruction、MCP、skills、sub-agent 定义管理接口。ACP 会话配置与 MCP 注入不能直接证明上述原生资源的 CRUD 能力。

先核对固定版本的公开契约和实际加载行为，再确定分能力的读写 API。关键判断是原生配置管理与 Velune 临时注入的归属、scope 与缺失表达、秘密字段、并发编辑与目录资源边界。不因某 Harness 不支持而伪造空列表或 silently 创建另一套资源。

## 分工与验证

runtime_design 负责关键架构判断；runtime_adapter 拥有 agent-runtime 契约、原生资源与本地／SSH 文件边界；runtime_application 拥有 application／bindings；claude_probe 负责 Claude／Pi 的固定版本加载证据与隔离验收建议；主执行者维护共享契约、长期文档、跨边界验收与提交。

验收须证明通过公共 API 写入的资源能被对应原生解析器或运行时发现，重读保留正文、附件和非目标配置；不存在、未支持、非法内容、路径越界、冲突及忙碌生命周期有可观察结果。至少覆盖本地、受管 HOME 和真实授权 SSH 目标，必要分支用合成 fixture。执行 fmt/check/clippy 和必要构建；不以静态检查替代动态文件边界验收。

开始时已有其它任务未提交：`docs/design/ai-service.md`、`docs/sources.md`、`scripts/build-macos.sh`、`scripts/manual-protocol-translation.py`、`app/mac/Licenses/RelayKit/`、`packages/ai-provider/relaykit-bridge/`、`tasks/new-api-reuse/`。不得纳入本任务提交；若需共享文档只编辑和暂存本任务段落。


## 已采用的实施契约

统一 Native-owned 的资源操作，不新增 Velune 镜像或热重载。ResourceKind 包含 UserInstructions、McpServers、Skills、NativeAgentDefinitions；User／Project 范围在执行机器解释。catalog 返回支持说明和目录条目；read 返回正文／二进制及执行位；write／delete 使用 Missing 或 Match(revision) 的显式预期。条目的存在状态与读写能力分开，错误保留可编程类别。MCP 的 key 是服务名称，仅该原生条目作为 JSON 读取编辑；其他类型 key 是限定类别根内的相对路径。

Skill 目录递归发现普通资产，既有资源按单文件原子更新或删除；仅允许删除空目录，不提供全目录事务替换。读取不跟随 symlink 或特殊文件。支持但未建的指令入口返回 Missing；空目录可返回空集合；不支持的种类或范围有独立说明。已有坏共享 MCP 配置不可被当空配置重建。读取不启动 Harness 或初始化 HOME，写入在 busy／协作 scope 存在时拒绝，停用实例仍可管理。

目标矩阵为 Codex／Claude 四项，Pi 前三项，DSH instruction／skills；DSH 的 MCP／subagent authoring 与通用 ACP 的原生资源 CRUD 在当前管理适配中明确 Unsupported，不宣称其 Harness 没有相应运行能力。原生布局来源、实际加载和安全文件操作仍由 owners 验证。

runtime_adapter 进一步把独立资源文件边界委派给既有 ssh_execution owner；自己持有原生路径／MCP section／公开契约责任。application owner 接线并准备 typed 错误；claude_probe 持有可复用 `scripts/manual-runtime-management.py` 与真实 Claude／Pi 加载验收，主执行者统一最终构建和采用。

## 完成状态与验收

2026-10-11，application／bindings 的公开 API 已完成隔离验证：四种停用的专用 runtime 可管理指令；缺失、未支持与 revision 冲突正确分类；未配置 user root 的 Project 管理可用；原生 Agent 定义保持字节；skill 附件保留二进制和执行位，非空目录不能删除；MCP 使用 opaque 服务名逐项操作。managed HOME 查询不创建目录，写入落到稳定的实例目录。busy turn 和协作 scope 阻止修改但允许读取，空闲 ACP 会话进程不会因管理操作被关闭。此组验收使用合成控制 peer，不能替代原生 Harness 加载证据。

非 UI 切片已完成。Codex app-server 实际发现所写 skills 和 Agent 定义，DSH 公开 loader 加载指令和 skills；Claude API 写入的指令与 skill 正文出现在 loopback Messages 请求中，原生 Agent／MCP 发现通过；Pi 真实 SDK 加载 User／Project 指令与 skills。本机 macOS 和真实 `wsl.win-ws.localhost` 的文件边界、权限、revision、目录上限和并发首次创建均通过。具体观察、固定版本、限制和可复现手动入口归 [native-evidence](native-evidence.md)，不把模型替身当真实供应商验收。

最终源码通过 `cargo fmt --all --check`、`cargo check --locked --workspace --all-targets --all-features`、`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`、bindings 的 no-default clippy，以及 `cargo build --locked -p velune-bindings --all-features`。Swift／Kotlin／Python 绑定均生成成功；本机未装 ktlint，Kotlin 自动排版未执行。最终动态库再次通过两份手动脚本的全部断言。未新增或运行自动化测试，未构建平台 App。

长期契约归 [architecture](../../docs/design/architecture.md)、[agent-runtime](../../packages/agent-runtime/README.md) 和 [运行说明](../../docs/development.md)。App 的资源列表、编辑器、冲突处理、敏感内容展示和生效提示均待实现；下次 Mac 更新生成绑定前，还须在 `Transport.swift` 补上新错误类别的穷尽匹配，本轮按用户要求不改 App。暂不承诺通用 ACP 私有资源 CRUD、DSH Cordis authoring、原生配置的全来源有效合并视图、热重载或目录事务。

并行任务的变更后来扩展到 ai-provider、gateway、协议转换验收脚本与 development 的 RelayKit 构建段落；这些均不是本任务改动，不得纳入本任务提交。Cargo.lock 仅采纳本任务 YAML／TOML 编辑和内容 revision 所需的依赖解析。

## Mac 接入（2026-10-11 起）

用户要求把上述管理能力接入 Mac App，主要在设置的 Agent 运行时页，并特别重视 UI／UX。用户决定（稳定内容已写入 PRD）：设置页主从布局，文本为主＋MCP 结构化，首轮只管用户范围，分批 骨架＋指令 → 技能 → MCP → 子 Agent。

### 第一批：骨架与指令

- 运行时页：左侧实例列表（保留添加、删除、快速导入、双击编辑、启停菜单），右侧详情，分段“概览｜指令”；设置窗口在此页放大。
- 指令：用户范围目录；不支持时显示原因；候选文件可选，未创建的文件保存即创建；纯文本编辑器（等宽、无智能替换）；未保存修改按文件保留，切换文件或实例不丢，关闭设置窗口会丢失；revision 冲突就地提示，可重新载入或覆盖保存；有任务运行时禁用保存；非 UTF-8 文件只读提示。
- 资源操作走独立后台队列，不占用全局 `isLoading`。

验证：Swift 严格构建；用临时 VELUNE_HOME（合成运行时实例与目录）启动构建产物，窗口截图确认；不读取真实配置。

第一批实现与验证（2026-10-11）：`RuntimeResources.swift` 提供指令模型（按文件保留未保存修改、revision 冲突、覆盖保存、创建／删除）、纯文本编辑器与主从布局。用 UniFFI Python 绑定在临时 VELUNE_HOME 建立三个停用的合成实例（Claude／Codex／通用 ACP，目录均在 /tmp），启动构建产物后经辅助功能操作确认：概览与指令分段；Claude 指令读取、编辑后出现“有未保存的修改”；外部追加后保存得到冲突提示，“覆盖保存”落盘；Codex 缺失的 AGENTS.md 以“创建”落盘（0600）；通用 ACP 显示不支持原因且头部固定在上方。过程中修正：缺失文件被误判为非 UTF-8、编辑区点击空白处无法定位光标、不支持页头部居中。临时文件已清理，未读取真实配置。已知限制：关闭设置窗口会丢弃未保存修改；删除流程只验证了界面入口，未在界面上走完。

设置窗口尺寸（2026-10-11）：（已被上一条取代）运行时页放大后与其他页尺寸不一致，切换页签时窗口跳动。改为 TabView 整体一个弹性尺寸（最小 760×520、理想 900×620，可由用户调整并由系统记住），各页内容填充；preview 窗口确认三页切换时窗口恒为 900×600。

页面组织修订（2026-10-11）：设置页被撑大后其他页松散，用户推翻“资源放在设置页”。现状：设置三页恢复 690×560 固定尺寸；Agent 运行时页回到紧凑实例列表并增加“管理资源…”（按钮与右键菜单）；新增独立 `Window("运行时资源")`（`NavigationSplitView`，可缩放，默认 920×640），入口还有 composer 运行时菜单“管理 <名称> 的资源…”和“显示”菜单“运行时资源…”，经 `RuntimeResourcesNavigator` 直达实例。preview 窗口确认设置页尺寸、资源窗口外观与菜单入口。

### 批量与梯级覆盖（2026-10-11 决定，待对齐后实现）

用户决定：Velune 资源库＋分层应用；层＝全局 → 自定义分组 → 单个实例，其中“全局”是真实存在的全局资源库（store）：频繁复用的 MCP、skill、子 Agent 在此集中定义，各层引用并可覆盖，不只是包含全部实例的分组（用户随后更正）；叠加＝按名称由更具体的层覆盖，指令不拼接，可显式在某层禁用继承条目。入口小改动已做：运行时编辑 sheet 增“管理该运行时的资源…”，设置页按钮同名。

草拟设计（未批准）：库条目带类型、名称和可移植内容；每实例的“有效集合”＝所属各层合并结果，叠在原生非受管内容之上。每实例在 VELUNE_HOME 保存同步清单（条目、路径、内容哈希、来源层）；应用＝期望集合对清单与原生内容求差，只创建／更新／删除受管条目，不碰用户手写原生文件。异构：指令与 skill 基本通用；MCP 保持同形（命令／地址、参数、环境、请求头）按运行时渲染 JSON／TOML；子 Agent 格式差异大，需可移植子集或仅在对应格式的运行时应用，不支持的运行时跳过并说明。

用户确认：全局 store 即“模板库”，语义为受管持续同步而非一次性复制快照。待用户决定：⓪全局 store 是否同时是默认应用到所有实例的一层（全局默认），还是只作为被引用的库；①一个实例属于多个分组时的优先级；②“实例层”是库中的覆盖项，还是直接编辑原生文件（当前资源窗口的行为）；③受管条目被原生侧改动（漂移）时：以 Velune 为准覆盖／采纳回库／忽略；④先做库与分层再做技能／MCP／子 Agent，还是先做完单实例管理。

### 模板库与受管同步：设计草案（2026-10-11，待用户复核）

用户决定（见 PRD）：全局模板库只作被引用的库；取消分组；漂移时用户显式选策略；采用 canonical resource definition；先做模板库与受管同步，再做各类资源界面。

**canonical definition**：每个模板有 id、kind、name 与类型化内容。指令＝Markdown 正文；技能＝以 SKILL.md 为入口的文件树（二进制附件、执行位保留）；MCP＝传输（stdio：命令、参数、环境；http：地址、请求头）与名称，环境值与请求头值可含秘密，只存 Velune 私有文件（0600），界面默认遮盖；子 Agent＝名称、描述、正文指令及可选的模型／工具提示。渲染器按运行时类型把 canonical 变成原生字节：MCP 写入 Claude／Pi 的 JSON 与 Codex 的 TOML；子 Agent 渲染为 Claude 的 Markdown＋frontmatter 或 Codex 的 TOML；运行时不支持该类（Pi 无子 Agent，DSH 无 MCP／子 Agent，通用 ACP 全无）或存在无法映射的字段时，跳过并给出无正文的原因，不伪造。

**存储**：`VELUNE_HOME` 下独立的模板库文件（带 schemaVersion，hard-cutoff，原子替换，0600），不并入普通配置，不随配置重置。每个实例另有同步清单：受管条目（kind、原生 key、模板 id、模板内容哈希、实际写入字节的哈希）。

**状态**（每个受管条目，由模板、清单与当前原生内容三方比较得出）：已同步；模板有更新（原生未动）；原生被改（漂移）；原生缺失；目标已存在非受管同名条目（首次应用冲突）。同步只通过既有 `management` 的读写 API，写入带 revision，因此外部并发改动返回冲突而不覆盖。

**用户显式选择**：首次应用冲突：接管（覆盖并受管）／跳过。漂移：以模板覆盖／解除受管并保留原生内容／暂不处理；“采纳回模板”因模板可被多个实例引用，默认不提供，需另行确认语义。原生缺失：重新创建／解除受管。模板有更新：预览差异并一键同步到所有受影响实例，不在后台静默改写。

**Mac 界面**：资源窗口的侧栏分“模板”与“运行时”两部分；模板按类别（指令、技能、MCP、子 Agent）列出并编辑；运行时详情的各类别列表同时显示原生条目与受管标记和状态，提供“应用模板…”（可一次多选运行时即批量）。

**待用户决定**：①指令是整文件覆盖，还是写入受标记块、保留块外手写内容（建议受标记块，同名模板替换其块）；②模板更新后是自动同步到已应用的运行时，还是提示后手动同步（建议手动并给出一键全部同步）；③“实例层”不做字段级覆盖，实例特有差异用该实例自己的非受管原生条目表达（同名冲突由首次应用时的显式选择处理），是否接受；④MCP 模板中的秘密存在 Velune 私有库并遮盖显示，是否接受。

设计草案复核（2026-10-11）：用户决定①指令模板整文件覆盖（不做受标记块）；④MCP 等秘密与 AI 提供商一致（Velune 私有文件，显式查看编辑，公开描述／日志／Debug 不含秘密）；同意③实例特有差异由该实例的非受管原生条目表达。②未明确回答，按建议先做手动同步（提示有更新、差异预览、一键全部同步）；“采纳回模板”默认不提供。用户允许在合适时新增 store package。

用户确认（2026-10-11）：不需要“采纳回模板”。漂移处理仅为：以模板覆盖／解除受管并保留原生内容／暂不处理。
