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

## Mac 接入与资源库（2026-10-11 起）

产品规则以 [PRD](../../docs/prd/index.md) 为准。历经“设置页主从 → 独立窗口按运行时组织 → 运行时＋模板两条轴”三版后，用户指出页面逻辑不对（同一资源出现在三处、概念过多、空状态未居中、侧栏展开收起卡顿），重新梳理并决定：以资源为中心；保存即同步；原生内容只能加入资源库后编辑。

### 已有基础（保留）

- agent-runtime：原生资源 catalog／read／write／delete（revision、写入闸门）；canonical 定义的 `render` 与尽力反向 `parse`、`fingerprint`。
- application：资源（内部仍名 `ResourceTemplate`）与绑定记录（`resource-templates.json`／`resource-bindings.json`，0600）、三方比较状态、应用／处置、从原生导入；`scripts/manual-resource-library.py` 手动验收。
- Mac：资源编辑表单（指令、技能文件、MCP 结构化与遮盖、子 Agent）、纯文本编辑器。

### 本轮重做（2026-10-11 完成）

- 核心：保存即同步（原生被改／被删的运行时跳过，忙时“待同步”）；状态为资源×运行时矩阵；“运行时中的其他内容”与“加入资源库”；删除预览接口。`manual-resource-library.py` 全部通过（未覆盖“忙时待同步”，需运行中轮次，留给用户在真实使用中确认）。
- Mac：`ResourceLibrary.swift`（窗口、模型、启用于、原生只读预览与加入资源库）与 `ResourceEditor.swift`（草稿、表单、技能文件、纯文本编辑器）取代原 `RuntimeResources`／`Templates`／`TemplateLibrary`；原生指令编辑器删除；运行时设置、编辑 sheet 与 composer 的入口改为“在资源库中查看…”，按该运行时筛选。
- 走查（临时 VELUNE_HOME、停用的合成运行时，窗口截图）：类别徽标与“有运行时需要处理”；资源详情的启用状态；原生被改后“用资源库的内容覆盖”落盘；子 Agent 的其他内容只读预览、加入资源库提示无法带入的字段并改写原生；勾选启用写入、取消勾选“保留原生内容”后条目回到其他内容；编辑描述保存后两个运行时同步更新；运行时筛选；新建技能。
- 未验证：侧栏展开／收起是否仍卡顿（截图无法判断，请用户确认）；关闭窗口时未保存修改的提醒仍未做（修改按条目保留在窗口内，关闭窗口会丢失）。
