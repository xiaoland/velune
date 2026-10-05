# Velune：共享协作指引

本仓库承载用户自有 Velune coding agent control surface，提供控制界面与协调层，不充当执行 Agent。Velune 是已确定的项目名；Beluna 是独立的、非 Coding 的通用 Agent / Digital Life 姐妹项目。

## 先读什么

1. `README.md`：入口与当前阶段
2. `docs/prd/index.md`：已确认的产品意图
3. `tasks/pi-mac-first-loop/packet.md`：当前任务、反馈修订和验收边界；Harness 研究证据留在 `tasks/harness-routing-feasibility/`
4. `docs/design/architecture.md`：已认可方向与具体技术推荐，区分已确认和待验证；不等于已授权实现
5. 需要核对决定出处时读 `docs/sources.md`

## 知识归属

- `docs/prd/index.md` 是产品目的、优先级、范围和稳定术语的唯一权威入口
- `docs/sources.md` 只保存来源定位和精简释义，不复制整段聊天或重复维护需求
- `tasks/` 保存易变的 Task Packet；问题、证据、假设、工作计划先留在任务里
- `docs/design/architecture.md` 归属跨单元方案与拟议契约，显式标记复核状态；证据快照与试验缺口仍留在 Task Packet
- 已有无凭据核心原型；实际运行说明归属 docs/development.md，不建立空的部署章节
- 将来出现独立知识归属与真实使用者，再依 SVC Corpus 引入相应规范；附近的 `AGENTS.md` 只补充本子树规则

## 必须保持的边界

- 核心优先级由 PRD 定义；不要恢复已被纠正的“手机优先／远程控制优先”路线
- “路由”仅指 LLM 请求路由；设备选择与 harness 调度不是同一个决定
- 首批 harness 固定为 Codex、Claude Code、Pi；以接管三者 LLM 路由为设计目标，研究实现路径，不把能力矩阵变成重新选择 harness 的闸门；排除 GitHub Copilot
- 已确认 Rust 主核心、SQLite、原生 Apple Swift／UIKit 与 Android Kotlin／Jetpack Compose 方向；不恢复 TypeScript 主核心或 Web 前端路线
- MCP 暴露协作工具、ACP 可替换接入已获认可；持久化是协作支撑，不代替发现、注入、委派、结果与失败协调机制
- Apple 具体平台覆盖、Mac Catalyst／AppKit、FFI 与运行宿主等尚有技术边界，不能把 native UI 当手机可以常驻运行桌面 Harness 的证明
- 应用配置使用文件系统持久化，根目录为 `VELUNE_HOME`，默认 `~/.velune`；AI provider 与 Harness 配置由平台装配／Host 管理，独立 AI lib 不自行读取全局环境，文件不保存秘密值
- 各平台尽可能采用所属平台的原生视觉与交互习惯；品牌及软件特点仅在图标、内容与少量细节中体现，不建立覆盖系统风格的统一皮肤。原生框架本身不等于原生体验；布局、控件、窗口、字体、材质、颜色和系统行为都应遵守该原则
- Apple app 使用原生组件与系统语义样式，不设独立深色模式开发配置或验收项；系统负责常规明暗适配，自定义内容仅针对具体显示问题检查和修复
- 不把聊天中的示例、助手提议或未核实的技术说法提升为已批准架构
- HAPI、Lody 只作说明性参考；Claude Code 订阅认证可研究其方案，但不意味着已允许复用凭据或认可可行性
- 不承诺订阅可以无限使用、免 token 计费、跨任意工具互通或规避供应商限制
- 不读取、复制或提交登录令牌、账户会话、密码、密钥、支付数据或真实会话记录
- 2026-10-02 已明确授权无凭据 Rust＋SQLite 核心及本地模拟验证；不推进 Factory26、不部署、不调用真实模型 API。本地提交允许；push、PR、真实认证和费用测试仍需独立授权
- 只采用 xiaoland/svc 的文档导航、知识归属和 Task Packet 方法；不要引入其 CLI、安装配置、运行时或其他额外工程
- 用户要求持久化仓库而非压缩包交付；本地 Git 提交不自动保证工作区或远端持久性

## 工作方式

当前早期阶段的所有重构采用 hard-cutoff：删除旧契约和兼容／迁移代码，仅支持新接口及配置 schema。遇到旧 schema 时直接初始化当前配置并原子覆盖，不保留旧配置或备份。原 Harness 文件、会话和平台秘密不属于普通配置重置范围。上游协议／SDK 的实际兼容参数不属于 Velune 旧契约，不因 hard-cutoff 而剥离。

- 开始非平凡任务前建立或恢复 Task Packet。先保持一个简短 `packet.md`，实际需要时再增长
- 严格区分：用户已确认的产品意图、已验证事实、待验证假设、候选方案、待用户决定
- 新事实给出来源、版本或提交、检查日期与适用边界；技术可行性和供应商规则以当时官方资料复核
- 用户作出新决定后，先更新唯一的长期知识归属，再更新 Task Packet，最后同步索引或短摘要
- 任务状态可以纳入 Git 以便跨环境恢复，但不是长期项目规范。任务关闭前确认持久结论已归位；获得正常删除授权后删除已关闭 packet，不另建任务档案库
- 共享规则放本文件；机器绝对路径、工具安装和临时操作记录不进入共享知识
- 文档编辑后检查相对链接和标题锚点，执行 `git diff --check`、`git diff --cached --check`，提交前核对 `git status`
- 核心改动执行 cargo fmt --check、cargo check --locked、cargo clippy --locked --workspace --all-targets --all-features -- -D warnings；运行方式见 docs/development.md。不需要文档 CLI

## 当前有界 AI service 工作

2026-10-03 用户明确限定：仅独立 AI lib／provider 配置边界的模块、契约、构造校验与文档。不新增测试、不运行既有集成测试；本步用 cargo fmt/check/clippy 静态检查取代上文通用测试步骤。不得推进网络、凭据、路由／fallback／重试、旧 Host／UI 改造。权威设计见 docs/design/ai-service.md，任务见 tasks/ai-service-contracts/packet.md。允许该任务新开发分支普通 push，不改 main、不 force push、不创建 PR。

2026-10-03 后续授权补充：允许在指定契约基线的新 dev 分支实现最小直接流式派发与 MiniMax provider，按任务预算进行合成 live fixture 采集及人工离线 replay。只读取已配置 Networksecret 占位供指定 HTTPS 请求，不输出／保存值。仍不新增或运行测试，不改旧 Host／UI，不部署，不调用 Ark／Bailian，不实现自动 routing／fallback。静态检查为 fmt/check/clippy；普通 push 已获授权，禁止 main／force／PR。预算及精确边界以 tasks/ai-service-contracts/packet.md 为准。

## Pi 与 Mac 首循环

2026-10-04 用户要求将上述分支合并 main，随后实施 Pi＋Mac 首循环，并明确至少拆为 core（AI 服务、Harness 适配器）与 app（Mac 等平台）。此任务提供 Chatbot 会话界面、会话列表和应用内通用资源配置；Pi 拥有会话持久化，Velune 只做 projection，不新建真实会话数据库。用户执行真实登录、调用和验收；开发方完成构建与隔离验证，不读取既有 auth／真实会话。上一节 MiniMax 的禁止 Host／UI 改造限于其有界任务，不阻止当前获准切片。后续持续迭代规则已授权自主提交当前任务；远端发布仍不由实现要求自动授权。任务状态见 tasks/pi-mac-first-loop/packet.md。

## AI 网关与运行时配置边界

model ID 指提供商 API 规定的模型标识；内部记录键和网关路由 alias 分别命名，不能让用户维护另一份“模型 ID”。模型仍是跨提供商存在的业务概念，提供商绑定保留实际标识及协议能力；同名不自动证明相同或可互换。

认证资源由 application 集中管理。提供商只保存已登记资源 ID，不持有 Harness 来源、文件路径或任意 Keychain 引用；gateway 只依赖异步认证解析契约，AI provider 只消费本次解析出的短生命周期认证。来源 adapter 与平台秘密设施在应用内装配，按登记的协议及端点授权目标。认证 UI 区分 API key／OAuth 与来源详情，Pi 不作为认证方式。普通文件不保存秘密，原来源文件和 refresh credential 不复制、不删除。

AI 服务不限于 LLM；当前 gateway 仅为 LLM Gateway，是 AI 模块的一种应用模式，直接 AI 消费不要求经过网关。原生执行按协议组织，服务商作为配置，不经 sampling 重建。业务与可观测性职责分离，usage／finish reason 可被两者消费。当前实施与边界以 docs/design/ai-service.md 为准，不将旧有界 MiniMax sampling 原型提升为通用协议架构。

AI 提供商与模型分开建模，模型跨提供商存在且拥有自己的参数；提供商关联模型。协议为枚举选项，支持 OpenAI ChatCompletions v1 与原生 Responses v1，暂不翻译协议。提供商、路由和 fail-over 归属 Velune AI 服务网关；Harness 只接注入的网关配置。用户可显式选择原 Harness 认证来源，网关委托该来源解析与刷新，不删除原配置、不复制 refresh credential，也不让执行 Harness 绕过网关。模型参数以提供商协议为权威，聊天示例不提升为每个提供商必须支持的字段。Agent 运行时区分类型与配置实例，同类型可配置多个独立实例，不能在 UI 或 Host 假定只有一份配置。

core 是跨平台 Rust lib，通过 ABI 嵌入平台 app；产品不得另启常驻 core／Host 进程或依赖 App↔Host socket。外部 Harness 子进程与本机模型网关保留各自的职责。Mac 修改后重新构建并安装至 `/Applications/Velune.app`。用户已授权开发方随时直接退出正在运行的 Velune：先正常退出，若应用拒绝退出，可终止已确认的 Velune 应用进程，无需再次要求用户手动退出；当前产品显示版本为 `0.1 beta.1`。

## 持续迭代协作

用户于 2026-10-04 明确采用持续迭代协作：开发方承担程序员责任，用户承担技术型产品经理责任。主动建立或恢复任务、持续维护 Task Packet 与权威长期文档，并在已授权范围内维护代码可理解性与可维护性，不等到最终交付才集中补文档。任务与范围随新反馈更新，不把已经纠正的实现当作后续固定前提。

协作采用敏捷开发：需求在实现阶段可以变化，不将已开始实现视为需求冻结。早期优先建立利于长期迭代的技术架构与开发基础，不以尽快产出替代关键设计。关键工程决策使用 advisor 梳理因果、约束和判别证据；涉及产品行为或实质范围取舍时向用户提出有依据的建议并请其决定。常规实现细节仍自主推进，不把每个技术选择升级为用户审批。

允许自主提交当前任务改动，无需每次等待提交指令；提交前核对改动归属与必要验证，不混入无关工作。此授权不自动包括远端发布或部署。产品方向、用户特定偏好和有实质影响的范围扩展由用户决定；已有授权内的实施与常规工程选择自主推进。

消息界面不显示用户或 LLM 的头像／昵称，右侧为用户、左侧为 LLM、中间为系统／Harness 状态；不得用 Velune 品牌冒充 LLM 作者。

## 验证原则

不保留或新增任何自动化测试。优先保证类型安全，执行类型检查、静态检查与构建。行为验收采用手动操作或临时脚本，尽可能走端到端路径，不新增单元测试或集成测试。临时脚本可放在 `scripts/` 下，不接入自动测试或 CI；真实提供商与产品体验由用户验收。既有自动化测试及仅供其使用的辅助代码、依赖和运行入口应删除。

## 独立 units 与跨语言接入

2026-10-05 用户决定全面采用 UniFFI，并授权拆分。共享 units 为 packages/ai、ai-provider、conversation、agent-runtime、gateway、application、bindings；平台 app 是独立 unit。configuration 为 application 内部模块，不建立 persist 或空 remote package。领域包不依赖 UniFFI，bindings 暴露具名类型 API；不恢复手写 C ABI／JSON dispatcher 产品接口。Mac 继续同进程嵌入 Rust，修改后重建安装。任务见 tasks/package-boundaries/packet.md，各 unit 入口见其 README。
