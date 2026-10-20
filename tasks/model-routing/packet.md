# 模型路由与自动 fail-over

2026-10-16 用户要求实现模型路由，验证场景为聚合两个 ChatGPT 订阅并自动 fail-over（[S30](../../docs/sources.md#s30)）。产品归 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，网关契约归 [AI 服务设计](../../docs/design/ai-service.md#路由fail-over-与生命周期)。

## 当前接续与凭据边界

当前为 2026-10-20 Kimi Coding Plan 实施，范围与状态见[末节](#kimi-coding-plan-实施2026-10-20)及 [Kimi 接入记录](kimi-coding-plan.md)。包括 Rust 支持、已有平台模板消费者和必要绑定重建；不含真实模型调用。

2026-10-16 首轮仅授权 Rust lib，前端另行安排，当时不改 Mac 源码或构建安装 app；后续 Mac 设置工作及本次 Kimi 最小消费者接入已有各自授权。

- 当前路线为 Velune 独立管理 Codex 订阅认证及 AI 网关聚合；正式 SIWC 留作未来申请后的独立接入。用户已知悉规则冲突并要求产品披露。
- Codex Switcher 只是两个验收订阅的来源，不做产品集成。用户最新明确授权直接使用既存两份认证：只在一次性验收进程内读取 access token/account_id，不读取 refresh token 值、不刷新或改写原文件。此前重新登录建议已被用户纠正。
- implementation owner：codex_source 负责 application 认证生命周期与 bindings API；subscription_transport 负责 ai-provider 与 gateway；subscription_manual 负责新隔离合成脚本；主执行者负责跨模块整合、既有脚本与权威文档及静态验收。其他并行任务改动不纳入提交。
- 基础订阅支持与真实双订阅验收已在 `dd5683d`、`2d852b9` 完成。2026-10-17 用户确认继续智能调度方案，Rust lib 已实现四策略、主动额度、使用上限、冷却单探测与会话黏性；前端仍不在范围内。

## 既有模型路由实现（历史基线）

设计文档把候选范围、切换条件与不确定提交下是否重复执行列为产品策略。以下为默认实现：

- 候选范围：只限路由的显式成员，按配置顺序；直接选择的提供商模型不 fail-over。成员必须同协议，这是必要条件，不是充分条件。
- 提交闸门：上游响应头或响应已转给调用方后不再切换；不拼接两个流。取消结束请求，不切换。
- 可切换：认证解析失败（本机来源不可用）、未收到响应的连接／传输失败或超时、HTTP 429、HTTP 5xx。
- 不切换：401／403（沿用“资格拒绝不换号”）、400／404／422 等请求错误、已开始流式传输后的中断。
- 冷却：429 使该提供商在 Retry-After 指定时间内（缺省 60 秒，最长 24 小时）排到候选末尾；全部冷却时仍按顺序尝试。冷却只保存在应用运行期间的内存中，跨每轮的网关 Runner 共享。
- 状态引用：请求带 `previous_response_id` 或 `conversation`（服务端状态属于某一账户）时只尝试首个可用成员。
- 加密推理内容等跨账户兼容性未验证，由真实账户验收确认。

## 实施

- gateway：`ModelRoute` 配置与校验；路由表按别名捕获有序候选；派发循环在提交前按上述规则切换；每次尝试一条分析记录（第二次起 request_id 加 `/n`）。
- application：持久配置新增可缺省的 `routes`，不提升 schema；路由键在模型解析、Pi 目录、原生注入、模型选择与轮次命名中按首个成员的能力参数使用；订阅能力取任一成员；删除提供商或模型时剪除成员，空路由删除。冷却状态 `RouteHealth` 由本机运行时持有并传给每轮的 Runner。
- bindings／Mac：`save_model_route`／`delete_model_route`；设置新增“模型路由”页（列表、双击编辑、拖动排序成员，＋ 只列同协议模型）。

## 验证与状态

- 静态检查：cargo fmt --check、cargo check --locked、cargo clippy（-D warnings）通过；`bash scripts/build-macos.sh --build-only` 零警告。不新增自动化测试。
- `scripts/manual-model-routing.py` 通过：
  - 网关矩阵：429 切换并冷却（第二次请求不再访问该提供商）、503 切换、401 不切换、连接失败切换、流开始后中断不切换、`previous_response_id` 不切换、直接模型不切换、全部失败返回最后一个、认证解析失败切换、ChatCompletions 入口转换到 Responses 成员后切换；每次尝试一条 analytics（`<id>/2`）；混合协议与键冲突被校验拒绝。
  - 应用：真实 Codex 0.159.3 与 Pi 1.0.2 在临时 HOME 中选择路由“订阅池”，两轮各自由第二个合成账户应答；第一个账户在整个运行中只收到 1 次请求（首次实现时冷却放在每个 Runner 内，每轮重复请求 4 次，已改为跨 Runner 共享）；轮次以“订阅池 · 模型路由”命名；混合协议提示“同一协议”；删除成员与空路由的剪除；空路由不写入配置。
- 同时修正三个手动脚本中的网关配置（新增 `routes` 字段与 `RouteHealth` 参数），`manual-gateway-analytics.py` 复跑通过。
- 设置页“模型路由”的界面没有在开发版中截图：打开设置窗口会激活应用，留给用户验收。
- 当时待验收项：两个 Pi 实例登录后的真实双账户验收。用户后续已否定依赖运行时的聚合方向，改为下文独立订阅 Rust 支持；不沿用该路径作为当前前提。
- 2026-10-16 安装：随提交 554007d 重建并安装到 `/Applications/Velune.app`。codesign 严格校验通过；构建清单的 source_commit 与该提交一致，dirty 为 false；后台启动后没有新的崩溃报告。

## 真实双订阅验收的归属（2026-10-16）

- 用户先要求用 `~/.codex-switcher` 的两个 ChatGPT 订阅做真实 fail-over 验收。核对后：该目录含两个账户的 OAuth（access/refresh/account_id）。本机权限分类器拒绝开发工具列目录，项目 AGENTS.md 亦明确「不能借 CLI token 冒充正式接入」「不读取、复制…登录令牌」「不…规避供应商限制」。
- 用户明确授权走该路线（读 token、以 Codex CLI 身份请求 ChatGPT backend）；实现过程被安全分类器中止。该路线最终判定为规避供应商访问边界 + 违反项目既定规则，不予实现；没有产生任何相关产品代码。中止前已读取账户文件（仅结构与过期时间，无值）并对 ChatGPT backend 发过少量真实探测请求；未写入、未刷新、未提交任何凭据；临时脚本与 Magpie checkout 已删除。
- 用户随后同意放弃该路线。真实双订阅验收仍待用户走官方路径：两个 Pi 实例各自官方「登录…」一个 ChatGPT 账户（Velune 只委托来源解析/刷新，不复制 refresh 凭据），导入为两个提供商后在「模型路由」页组成路由。
- 讨论但未决定的替代方向：**运行时层**的 Codex app-server 多账户聚合（两个 Codex 实例各自官方登录，Velune 按用量做 turn 级 fail-over，会话绑定账户、不中途换账户）。这与本任务已实现的网关层模型路由是不同层次的能力，属产品范围扩展，另需用户决定后再开任务。

## 本轮方案研究（2026-10-16，待技术方案确认）

用户已确认 AI 服务独立多订阅聚合的产品行为，并允许参考 Magpie 与 Codex Switcher。本轮不改源码、不读取真实凭据、不调用认证或模型端点。上文真实账户探测记录属于既有其它轮次，不作为本轮操作或成功验收证据；其中运行时聚合候选不符合用户最新确认的方向。

建议保持现有 units：提供商实例拥有独立订阅认证，application 装配登录、私有持久化、串行续期与失效状态；ai-provider 负责原生订阅请求；gateway 负责有序模型候选、冷却与响应提交闸门。Agent runtime 只是调用方，不作为订阅注册前提。不新增中央账户 CRUD。详细认证路径仍待确认。

公开代码研究固定 Magpie `9b04c4e20ff8bf10c745a53f562f5007b82d007d` 与 Switcher v0.2.20 `9dec3c18a856abbb545241ae867c5b5ba9d92fa6`。Magpie `internal/provider/account.go` 以账户提供请求签名、模型目录和订阅 backend，`internal/gateway/fallback.go` 区分失败候选与请求错误；可借鉴职责分离，但其 401/403 等宽泛切换不是已批准 Velune 策略。Switcher 的认证模块示范登录、刷新及持久化协调，不引入其活动账户切换机制。

认证路径存在实质缺口：Switcher 原生 Codex 客户端认证与正式 SIWC 的客户端注册、scope 和推理目标不同，不能互换。SIWC 可支持 Velune 自身登录，但 [2026-09-29 SIWC 条款第 4 节](https://openai.com/policies/sign-in-with-chatgpt-terms/#4-prohibited-activities) 明确限制账户轮换及订阅用量池化，不能把它当作双 ChatGPT 订阅自动 fail-over 已受支持的证明。此限制限定该接入路径，不推导成所有提供商聚合禁止；也不自行将验收替换为 API key、手动切换或运行时聚合。后续须明确实际采用的 ChatGPT 认证与请求协议及其技术可行性，再确定实施方案。用户随后明确知悉 SIWC 规则冲突，要求保留目标并在产品中披露，由使用者决定是否承担使用风险。规则冲突已成为知悉的产品约束，不再等待用户重复确认风险；不把开源身份作为免责或供应商认可的依据。源码实施仍等待具体方案确认。


最新确认：当前采取 Magpie／Switcher 的 Codex 订阅接入思路，正式 SIWC 待未来申请后另行实现。已向原认证研究 owner 追问具体公开协议与 Rust 边界，向原 advisor 追问认证失败、续期与跨账户状态策略。仅研究及文档修订，下一步交付可确认的具体方案。


## 已确认实施方案（仅 Rust lib）

1. 每个提供商实例可选择 Codex 订阅认证，在 Velune 打开浏览器分别登录。沿用公开参考实现的 OAuth + PKCE、随机 state、本机 1455 回调；该路径复用 Codex OAuth 客户端身份及 Codex backend 请求约定，不宣称 Velune 正式 SIWC。端口占用直接提示，不采用参考实现间尚有冲突的随机端口回调。
2. application 管理提供商私有认证及串行续期，ai-provider 用具名 Codex 接入 profile 处理 backend 请求和限制；gateway 只消费本次 access credential 及账户标识，不接触 refresh token。复用提供商认证持久化边界，不增加中央 Account CRUD。原子替换轮换凭据，以 generation 阻止删除或重新登录后的旧任务回写；已发送的刷新不随 turn 取消而丢弃结果。
3. 路由按已有顺序使用成员。过期凭据请求前续期；401 在同账户强制恢复并重发最多一次，仍失败则标记需登录，未提交响应时可用下一成员。这是本提案的策略选择，不是现有实现或供应商契约。403、400/404/422 等权限或请求错误停止；429、明确结构化额度错误、5xx、连接失败及超时允许提交前切换。429 沿用有界冷却，不从错误文案猜测额度。流交付后中断或用户取消不切换。
4. 聚合路由只接受带完整必要历史且不引用上游账户状态的请求；previous_response_id/conversation 明确报不支持，不静默删除或改发另一账户。现有先按冷却排序再截取一个候选不能保证状态归属，需修正此行为。跨账户 encrypted reasoning、文件引用及工具历史仍需真实验收，不承诺所有状态可移植。
5. Mac 复用提供商编辑与模型路由页面，显示订阅登录状态、需重新登录、成员失败与切换原因。接入及路由配置披露非正式接入、供应商限制和使用风险；正式 SIWC 留作以后独立接入，不预建未使用框架。
6. 建议用户在 Velune 为相同两个订阅分别重新登录，避免与 Switcher 共享 refresh token 的轮换所有权。Switcher 目录不是产品依赖；不读取或复制其真实认证，不新增导入产品。若用户必须直接复用原有 credential，则须先决定单一刷新所有权及可执行的验收方式，不能声称仅复制文件不会影响原登录。

静态验证按仓库 fmt/check/clippy 与 Mac 构建安装要求；合成隔离脚本验证并发续期、重试上限、失败分类、提交边界和状态引用拒绝，不引入自动化测试。真实双账户由用户登录及验收；尚未获得真实协议行为证据，源码仍未实施。


2026-10-16 用户已授权本节方案的 Rust lib 实施；此前“待确认／未改源码”段落保留为历史研究记录，不代表当前授权状态。Mac 前端与安装不在本轮范围。


## Rust slice 实施与验证（2026-10-16）

已贯通 application 独立 Codex 登录／私有持久化／续期、ai-provider 具名订阅 profile 与受控 ResponsesClient、gateway 有界 401 恢复及失效切换、bindings 的认证编辑／提示／账户尾号。认证仍归提供商，不新增中央 Account CRUD。前端可按 [bindings README](../../packages/bindings/README.md) 接入；Mac 源码、生成 Swift、app 构建安装均不在本轮。

隔离手动证据：既有 `manual-model-routing.py` 的 gateway 阶段通过 11 个场景；新增 `manual-codex-subscriptions.py` 在临时源码副本中仅重写认证／backend 固定目标及 resolver 可见性，执行时 HOME 与 VELUNE_HOME 隔离、未替换远程 URL 由死代理阻断。它覆盖 PKCE/state、独立订阅登录与重开、12 并发只旋转一次及原子读取、目标绑定、429 冷却与切换、401 恢复上限／analytics 最终状态／失效切换、聚合状态引用拒绝、流提交边界、临时续期失败后恢复、已发送续期在 caller 取消后仍提交、重新登录保存失败保留旧配置和会话、交互式登录互斥与 302/307 不跟随。该证据不证明真实 OAuth／TLS、供应商资格、真实模型与跨账户历史兼容性。

新增脚本最终 15 项通过，另核对 notify／url 登录事件契约和公开账户尾号／认证描述脱敏。真实双订阅与前端验收仍由用户后续完成，不宣称已经达到真实账户验收效果。

最终源码检查以提交 `4adf1cb` 加本任务改动的隔离副本为准，排除共享工作区正在开发的 runtime work lifetime 改动：`cargo fmt --check`、`cargo check --locked --workspace --all-features`、`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`、`cargo build --locked -p velune-bindings` 均通过，后三项离线执行。application／bindings 的 configuration-only 构建同时保持可用；纯公开 SendQueueOperation 类型从 local 模块移入 API，形状不变。未执行自动化测试、真实认证或模型请求，未修改前端、未生成 Swift 或安装 app。


## 真实验收接续（2026-10-16）

用户要求进行真实验收，并询问历史兼容性。本轮获准发起真实双订阅验收，但不读取此前被工具拒绝访问的 Switcher／原 Harness 认证目录，不通过新 helper 绕过该拒绝。使用本次 Rust 接入流程，在隔离应用 HOME 中由用户在浏览器分别登录两个账户；工具不接收、输出或记录令牌与密码，验收入口只输出阶段、状态、布尔判定和必要用量。

验收冻结源码提交 `dd5683d`，排除工作区其它任务的未提交修改。最多 4 个逻辑推理请求，产品 401 恢复可能使实际 HTTP 推理尝试增至 8 次；不自动重跑整套，不使用 API key 或其它付费 fallback。仅使用短小合成输入，每请求等待上限 60 秒。订阅 backend 不支持输出硬上限，短输出由提示约束，不能将其写成服务端强制限制。

计划先验证 A 的实际工具调用与 B 的短文本响应，再把 A 的真实合成历史和实际执行的无副作用工具结果交给 B 续接。最后在首候选注入一次本机 429，第二候选仍请求真实 B，验证网关切换。不能把合成 429 宣称为供应商真实限流；不故意消耗额度制造限流。文件 ID、服务端状态引用及加密推理的任意跨账户可用性不作预先承诺。

当前状态：验收 runner 准备中，尚未真实登录或调用模型。必要的人机操作是用户完成两次浏览器登录；无需再次确认已经批准的功能方案或使用风险。


用户随后明确纠正验收方式：不要重新登录，直接使用 `~/.codex-switcher` 保存的两份订阅凭据。本轮按该新的明确授权通过原执行工具正常进行只读预检，未借其它工具隐瞒访问，工具调用成功。仅输出两个 chat_g_p_t 记录、账户绑定不同、两份 access token 的本地有效期检查通过；未输出 token、账户 ID、邮箱或原始认证 JSON。历史拒绝记录不能当作本轮工具结果，也不据此虚构新的永久禁令。

真实 runner 改为一次性只读来源：仅内部消费 access token/account_id，不复制 refresh token、不刷新、不改写 Switcher／Codex 文件、不向 application 私有存储导入现有会话。这是验收环境适配，不形成产品 Switcher 支持。原计划的浏览器登录与 application 真实 OAuth 验收取消；当前真实验收只覆盖 provider/gateway 双账户调用与切换。若实际 401 则停止对应真实调用，不争用原来源的刷新所有权。

真实诊断进展：最初所选 gpt-5.5 不在两账户目录，返回 400；改用共同公开模型 gpt-6.1-sol。随后上游 200 未形成可解析 SSE。已修正网关将提交前协议失败误报下游 200 的问题（现在 502，analytics 仍记录真实上游 200，不因此扩大切换资格）、SSE MIME 比较与 Codex 单一 Accept。修复后安全诊断确认实际响应缺少 Content-Type 和 Content-Encoding；继续检查正文分类，不将 HTTP 成功当作业务完成。所有请求只含合成短输入，无真实历史；自然额度耗尽未制造或验证。

受限正文诊断（同一只读来源、单个 A 请求）观察：HTTP 200、35 个 headers、无 MIME／编码声明，首块截取 961 字节且仅分类为 SSE。未输出正文。该证据支持仅对 Codex 缺失 Content-Type 的真实流采用现有 SSE 解析；仍要求显式成功终态，其他 profile 与显式错误 MIME 不放宽。首块读取仅用于诊断，最终代码删除，避免错误路径额外等待。

## 真实双订阅结果（2026-10-16）

仅使用用户指定的两份既存认证，账户绑定不同，未刷新、复制 refresh credential 或改写原文件。真实模型目录两次 GET 均 200，选择两边共同公开的 `gpt-6.1-sol`。初始六次推理诊断中，前两次选用目录不存在的 gpt-5.5 返回 400；随后四次请求在 MIME 检查处失败，其中最后一次受限首块诊断证明真实 SSE 没有 Content-Type。没有把这些 HTTP 200 计作成功。

最终两次真实推理通过：

- A 极简文本请求：HTTP 200、显式 `response.completed`、analytics Completed 且 usage_complete=true；精确返回合成 nonce。用量为输入 34、输出 16、合计 50 tokens。
- 路由请求首候选在回环服务注入一次 429，只使用固定合成凭据，未向回环服务发送真实 token。第二候选真实 B：HTTP 200、显式 `response.completed`、analytics Completed 且 usage_complete=true；从 A 的实际普通文本历史准确取回 nonce，当前追问不重述标记。输入 73、输出 16、合计 89 tokens。

最终生产源码为 `dd5683d` 加两文件 SHA256 overlay：`responses.rs` 为 `9ce0fa04844ebf1ad49fb18cf96422be3714700f217bcfcf42043060fcd0fcbd`，`runtime.rs` 为 `d86231ce835bf78f0529aa722b454bece33071a41727ebfab186cfe2d854857f`。无端点或认证可见性改写；真实验收全程累计 8 个推理 HTTP 尝试和 2 个目录 GET，不含本地注入。初期失败调用是否产生供应商计量不可从失败响应确认；不据此声称零消耗。

修复保留原始 SSE 字节，仅对 Codex 真正缺失 MIME 的成功流补下游类型；显式错误／无效 MIME 与普通 Responses 不放宽。流提交后若空流、HTML、缺终态或断流，仍失败且不切换。删除临时首块诊断读取，不在普通错误路径等待正文。最终整工作区 fmt、locked/offline workspace all-features check、all-targets/all-features clippy（-D warnings）与 velune-bindings build 全通过；没有新增或运行自动化测试，没有改动 Mac。

验收边界：这证明真实双订阅在故障注入下可切换及普通文本历史可接续，不证明自然额度耗尽、加密推理、工具历史、文件或服务端状态跨账户可移植；application 真实 OAuth／续期未运行。产品无 Switcher 导入或后台共享刷新能力。

最终合成人工矩阵 20 项通过，新增 Codex 缺 MIME 的有效 SSE、空流／HTML／缺终态失败且不切换、普通 profile 缺 MIME 与 Codex 显式空 MIME 拒绝；live parser 5 项本地自检及 usage 白名单自检通过。真实响应出现新的 usage.attribution 对象，验收 runner 已改为仅输出五个固定 token 数值字段，不传播未知对象或上游消息 ID。未再为日志收口重复真实请求。

## 智能调度接续（2026-10-16，方案阶段）

用户要求增加智能调度、主动额度感知、使用上限、冷却降序与会话黏性，并核对一个订阅对应一个提供商实例。该实例归属与原实现一致，已归位 PRD；继续限定 Rust lib。沿用用户先确认方案的要求，本轮调查及编辑文档，不修改产品源码或读取真实凭据。advisor subscription_path 负责审议五项能力的选择顺序、额度未知、硬限制与黏性冲突，以及中立额度／会话身份契约。

当前证据：gateway 只有有序 ModelRoute 和按provider记录的429内存冷却，没有额度快照和会话黏性。公开Magpie固定源码9b04c4e的subscription_usage.go展示Codex额度窗口采集，但Velune尚未真实验证该来源。候选方案以资格过滤→黏性→策略排序→有界失败切换为一条流水线，避免多个机制各自决定候选。具体默认值及未知额度行为待方案收口。

待用户确认的具体方案：一个订阅仍对应一个 provider 实例，模型条目可多个；实例统一拥有认证、配额快照和各窗口百分比使用上限，所有路由与直接调用共享限制。路由提供 order/smart/rotate/weight，smart 在资格和冷却过滤后优先合格会话绑定，否则按适用窗口 min((cap-used)/cap) 降序，配置顺序破同分。该分数不等于不同订阅绝对token容量。advisor建议仅order/smart；主执行者保留rotate/weight作为用户所请求更丰富调度中明确可解释的均摊与比例分配能力，不引入pace预测或单独least-used重复策略。

application按需启动有界刷新周期采集Codex窗口，用中立只读契约供gateway消费，显式区分未知/过期/不支持；不将本地tokens统计冒充供应商额度。配置上限而无法获得新鲜观测时暂停该实例；无上限未知实例降序候选。到重置时间仅触发重新观测。冷却期不供普通尝试穿透，全部冷却返回最早重试时间，期满单请求探测，恢复正常排序，不附加永久沉底状态。会话绑定采用调用方显式不透明ID，跨Runner共享；没有ID不猜测，首版运行期有效，替补成功后重绑，健康/上限优先。沿用现流提交闸门。上限是依据观测暂停新请求的阈值，不能承诺外部并发和在途请求精确不越线。尚未实施，等待本方案确认；不涉及Mac前端。

## 已授权智能调度实施（2026-10-17）

用户回复“确认”，上一节方案转为实施依据。PRD及跨单元设计已更新。transport owner负责gateway配置、额度只读契约、共享调度状态和派发；codex_source负责application额度生命周期/配置/session装配及bindings；manual owner负责人工脚本与app/host必要契约适配；主执行者负责跨模块整合、长期文档、整工作区静态构建和交付。advisor只处理关键跨单元判断。均不修改Mac或生成Swift。

验收应从实际HTTP网关与application配置边界证明四策略的分布和资格、阈值覆盖直接调用和所有路由、未知/过期及reset行为、额度并发更新与旧账户结果失效、全冷返回重试时间、半开单探测和取消释放、跨Runner会话黏性及替补成功重绑。使用临时合成服务，不新增自动测试/CI。既有真实来源只读授权保留，但本阶段优先隔离验收，不消耗真实额度制造限流。最终执行fmt/check/clippy、bindings构建与configuration-only检查。


实施收口事实：额度来源采用按查询／使用触发的一次异步采样，provider single-flight、服务最多 4 个采样任务；槽位满时立即返回现状，调用方后续查询重试，不建立持续轮询或后台队列。窗口 ID 采用供应商窗口秒数，显式保留 allowed=false；缺失百分比不填零。普通 token 续期保留身份 generation，重新登录或更换认证使旧 quota 视图、在途结果与会话绑定失效。重复订阅登录按 subject 与 workspace/account 组合拒绝，避免同一订阅拆实例绕过限制。

人工配置重开验收发现保存路径仍写旧 schema 8、加载路径已要求 9，导致刚保存的配置重开被重置；已改为 repository::CURRENT_SCHEMA_VERSION 单一常量供读写共同使用。该缺陷由隔离 HOME 的实际保存／重开暴露，静态检查无法证明持久化行为。当前工作未读取或重置用户现有配置。


最终 Rust 静态与构建检查：`cargo fmt --check`、`cargo check --locked --offline --workspace --all-features`、`cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings`、`cargo build --locked --offline -p velune-bindings`、application／bindings 的 `--no-default-features` check 均通过。未运行自动化测试、真实额度接口或模型请求，未修改前端、生成 Swift 或安装应用。长期文档相对文件链接及标题锚点已检查，无缺失目标。


合成人工验收结果：`python3 scripts/manual-codex-subscriptions.py` 的 application／bindings 24 组通过，覆盖独立认证与原有流式边界、新额度 Fresh／非法负值拒绝、阻塞的旧额度请求换认证后不回写、四策略与上限保存重开，以及 bindings 门面的保存／重开与 Unsupported。`python3 scripts/manual-gateway-analytics.py` 的 14 组及原始字节保真通过；原模型路由 gateway 矩阵 12 场景通过，新增真实截断上游响应后的后续直接调用冷却。仅编写临时合成 runner，不新增测试框架或 CI。

真实验收脚本已删除历史双文件 overlay，改为启动时解析 `--source-ref`（默认 HEAD）并冻结完整提交；默认只编译和运行本地 SSE parser 自检，不读取认证或调用网络。旧真实结果仍对应前文已记录的历史提交与文件摘要，不借用作新额度能力的真实证明。


`python3 scripts/manual-smart-routing.py` 的最终基础矩阵 12 组通过，覆盖四策略跨 Runner 的 128 次分配、归一化余量、直接调用／多路由共享阈值、未知／过期／重置、显式 namespace 与路由范围粘性、失败或未完成不绑且无 analytics sink 同样成立、成功替补重绑、12 并发单探测、取消释放、认证 epoch 失效及全冷最早 Retry-After。最早时间场景让候选分别冷却 5 秒和 60 秒，确认返回较早机会且没有提前调用冷却中的候选。


补充同窗口 ID 的全局与模型专属观测回归后，智能调度矩阵共 13 组全通过：全局已用 10%、模型 A 专属已用 99%、使用上限 50% 时，A 不得发出上游请求；只适用全局窗口的 B 正常完成。gateway 对所有适用的同 ID 窗口逐个检查，不能由第一个低使用窗口掩盖另一个窗口。修后 gateway fmt/check/clippy 与完整 bindings 构建再次通过。历史人工脚本仅同步新必填 quota_limits／路由构造字段，未声称重新验收全部外部 Harness 场景。

本阶段 Rust 实现与隔离验收完成。额度观测尚未对真实 ChatGPT 端点验收；上限仍是观测驱动的请求准入，不能保证在途或外部并发不越线。前端继续由用户另行安排，需按 bindings unit 新契约接入。


实现提交为 `8616bd0`。提交后运行 `python3 scripts/manual-codex-subscriptions-live.py`，完整冻结 `8616bd0a95dabc9e11db1cf63203a497a2f38c57` 的 runner 编译成功，5 个本地 SSE fixture 与 usage 白名单自检通过，real_requests=0；未传 `--run`，未读取凭据或执行任何计划中的真实请求。


## 订阅能力解耦研究（2026-10-17）

用户确认推进协议组合解耦，并要求先弄清 subscription 特殊性、研究 Magpie，下一步拟接 Claude Code 订阅。本轮调查与方案收敛，不改产品源码、不读取凭据、不调用真实认证或模型。现状是 provider 和 LLM 协议通用、gateway QuotaSource 中立，但 application 额度绑定 CodexSubscription，Responses 内有专属目标/headers/MIME，gateway 的401恢复也识别专属profile；不能声称已经完全组合。

transport owner 固定公开 Magpie 源码研究认证、请求、额度与品牌耦合；codex_source owner 研究 Claude Code 官方边界与本地认证/额度接缝；主执行者整理跨单元方案与独立判断。调查需回答：subscription究竟改变哪些协议、哪些能复用、哪些需专属adapter，以及能否用Codex和Claude两条路径验证组合边界。方案未确认前不扩大为动态插件系统或任意脚本化协议配置。


研究结论与证据归 [订阅组合研究](subscription-composition.md)。Magpie 当前固定 HEAD `dccfefdf` 的 Claude 订阅执行、认证和额度均依赖 Claude Code CLI，不能作为直接订阅HTTP路径的证据。advisor确认聚合仍可归gateway，但CLI成员是实质执行后端扩展；候选为先解耦现有HTTP路径，或显式纳入CLI会话／工具桥。尚待用户选择Claude执行路径；未改产品源码或做真实调用。


## 组合解耦与 Claude 后端实施（2026-10-17）

用户明确确认实施并接受本机 Claude Code、进程、会话和工具桥。由于共享主工作区有其他任务正在修改runtime/application，本轮在附加的 subscription-composition worktree 从 deec45d 开始，不能覆盖或混入另一任务。transport负责gateway及既有ai-provider协议/认证恢复；claude_backend负责独立Claude执行与工具桥模块；codex_source负责application/bindings配置、CLI认证与额度；manual负责合成端到端验收；主执行者负责文档、跨边界整合与最终验证。advisor处理执行提交/取消与能力组合契约。

验收边界：一订阅一provider；协议、backend、requestprofile、认证、额度由有效组合装配。Codex旧能力不退化；Claude使用独立配置目录和显式会话身份，CLI持有秘密与刷新，不读取用户现有登录。工具仅返回调用方执行，下一请求工具结果恢复，取消释放挂起与子进程；旧绑定结果无效，可能已执行的CLI请求不得透明重放。通过隔离HOME下合成CLI、回环HTTP/MCP与保存重开验证，不新建自动tests、不调用真实CLI/认证/模型，不修改Mac。最终fmt/check/clippy/build/configuration-only检查；当前用户工作区并行改动另行保留。

实施已贯通 ai-provider、gateway、application 和 bindings。配置 schema 10 使用同一常量读写，构建 manifest 从此常量提取；旧普通配置按既有 hard-cutoff 重置。Pi 原先按路由中任意订阅整体剥离参数的 hook 已删除，请求限制由实际选中成员的 profile 拥有。Claude CLI 使用独立运行时维持跨 Runner 会话，工具只通过私有 MCP 交给调用方；取消与绑定失效终止进程，显式新用户轮可恢复，取消的旧调用 ID 仍拒绝。

合成人工结果：新组合先完成 17 组，再补取消恢复 1 组，共 18 组通过。取消补验覆盖 pending 时不能开启新轮、原 PID/MCP 退出、新轮新 PID 成功、迟到旧工具结果返回 409 且 CLI/HTTP fallback 计数均不增加。Codex/application 24 组、智能调度 13 组、analytics 14 组与原字节保真回归通过。59 份人工脚本及合成 CLI 的 Python AST 通过。没有真实 Claude CLI、登录、模型或额度调用；真实版本兼容和订阅资格仍待真实验收，不将合成结果等同供应商支持。

全工作区 `cargo fmt --check`、`cargo check --locked --offline --workspace --all-features`、`cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings`、`cargo build --locked --offline -p velune-bindings` 通过；application/bindings 的 no-default-features 检查通过。更改 Markdown 相对文件链接、build-macos shell 语法与 diff 检查通过。仅调整构建 manifest 的 schema 来源，没有构建或安装 Mac app。

补充边界：外层 Harness 真正结束时调用 `finish_user_turn`，终止仍运行或挂起工具的交互，保留 Completed idle 进程；不把普通运行时准备当作结束。登录 worker 在取消、未提交丢弃或配置提交失败时有界执行官方 logout，再删除暂存目录；结果已经送达但尚未提交时取消也立即触发清理。两组清理合成验收通过，证明原配置与原目录保留、暂存 logout/删除发生且取消不依赖应用对象销毁。最终生命周期修复后全工作区 fmt/check/clippy 和 bindings build 再次通过。

最终新增组合证据共 22 组：先前 17 组、最后 `--case cancel` 的取消/迟到结果与 finish Pending/Completed 共 3 组、`--case auth-cleanup` 2 组。Pending 结束后原 PID 退出且新轮成功，Completed 结束后跨 Runner 仍复用同 PID。清理 case 仅在临时源码副本插入发送结果后的观测 marker，以精确复现“成功已到但尚未提交”，不改变产品分支。最终脚本 AST 与 diff 检查通过。

实现提交 `57bf2ce6454fad2313f6cfa46468e16c9989ffce` 后执行默认 compile-only 的 `manual-codex-subscriptions-live.py`：完整冻结该提交并成功编译 runner，本地 SSE parser 5 例和 usage 白名单检查通过，退出码 0、real_requests=0；未读取认证、刷新或发起真实调用。实现留在 `feat/subscription-composition` 独立工作树，未合并共享主工作区的其他任务、未 push 或创建 PR。

## Claude Code 真实接入推进（2026-10-17）

用户要求继续推进订阅接入。本轮沿独立组合分支核对本机官方 CLI、准备 application 登录与 gateway 端到端人工 runner，使用新隔离配置目录，不读取或复制既有登录令牌。只进行有界文本/工具往返及一次额度观测；登录由用户在官方浏览器完成。主工作区目前 main 已有其他提交，不将旧分支覆盖主工作区。现有合成验收不能代替真实协议兼容证据。

本机空 HOME 下探测 CLI 版本为 2.1.292；auth login/status 的本机 help 与当前官方 CLI reference 一致。状态 JSON 只白名单输出登录状态、认证方式、目录一致性与字段名，不输出账户内容。

真实接入 runner 已冻结 `da991c2b1942f9ec35e89ecf2cf849fcea0c2783` 编译，通过 3 个本地 SSE 解析自检。通过 application 启动官方 CLI 登录，模型请求为 0；等待用户完成浏览器授权。脚本只输出白名单阶段、退出状态与计量，不输出原始 CLI 内容或账户信息。登录不等于推理、工具或额度已验收。

首次真实登录：用户完成浏览器授权，但报告找不到钥匙串 unknown，application 返回 LOGIN_NOT_COMPLETED，模型请求为 0。环境判别仅调用 `/usr/bin/security default-keychain -d user` 查询路径元数据：真实 HOME exit=0 且有路径；临时 HOME 在保留 USER/LOGNAME 时仍 exit=1，无路径。未读取钥匙串条目或秘密。本机 2.1.292 发布二进制显示 Keychain service 随 CLAUDE_CONFIG_DIR 的 SHA256 前八位区分，账号名取 USER 或 os.userInfo().username；不能套用旧版 unknown 账号 fallback 报告。修正方向为保留系统 HOME/身份、只隔离 Claude 配置目录，并对认证失败增加安全阶段诊断。此结果证明伪 HOME 破坏默认钥匙串发现，登录恢复仍需真实复验。

HOME 修复已覆盖 application 登录/状态/退出/额度与 ai-provider 执行，保持环境白名单、配置目录和工作目录独立。新增固定 phase/reason/exit_code 诊断，未扩展 UniFFI 类型。合成 cancel/finish 3 组及新增认证诊断 3 组通过；每个 CLI 子命令验证父 HOME/USER/LOGNAME 保留及独立配置/cwd，故障输出含 sentinel 仍不泄露。新增组合累计 25 组。live runner 的 9 项解析/诊断与 2 项失败输出自检通过。fmt、workspace check/clippy、bindings build 与 no-default-features check 均通过。

修复提交 `aca0a8e938c3a0fe78183f48b4f276a6ce6bde91` 冻结后第二次真实官方登录成功：application 返回 login_complete/configured=true，模型请求 0，未读取或复制 token。由此补足真实登录证据；下一步独立执行文本、工具、额度阶段。

同一冻结提交的真实文本阶段只发送 1 次网关请求：CLI 结构化 rate_limit 映射为 HTTP 429，analytics=Failed、用量未报告，没有自动重试。随后独立 quota 阶段仅触发 1 次观测，Fresh，5 小时窗口已用 100%、周窗口已用 53%，problem=false；未发送工具验收请求。此证据支持登录与当前 CLI 额度输出解析成功，以及限流错误传播，不证明正常文本或工具往返成功。

用户明确选择保留登录，额度恢复后继续。本轮停止真实请求，不退出该订阅、不删除验收配置。待完成：用相同专用验收目录先执行 live runner `--run --stage text`，成功后执行 `--run --stage tool`，不重新登录、不自动重放失败请求；仍不扩展前端或合并主工作区。本轮模型网关请求共 1 次（429）、显式额度采样 1 次；CLI 内部网络次数未计量。

## 合并回 main（2026-10-17）

用户明确授权将订阅组合 worktree 合并回主工作区 main。合并前 main=`f10565c`、订阅分支=`008c67e`，主工作区干净。三处文本冲突按双方契约解决：普通配置 schema 10 与主分支 organization schema 2 共存；保留运行时 compatible_versions 文档并更新普通 schema；人工 Swift fixture 保留主分支 pi-1.0 ID 并补齐四个 provider 组合参数。未改 Mac UI、生成 Swift 或安装应用，未触发真实认证/模型/额度请求；保留已登录订阅，真实文本/工具验收继续等待额度恢复。

合并树验证通过：cargo fmt --check、workspace all-features check、workspace all-targets/all-features clippy -D warnings、bindings build；合成订阅组合默认 20 组全通过（real_cli_executed=false），61 份人工脚本 AST 通过。未重复已经通过且未受冲突影响的真实调用或其他回归。

## 额度恢复后的真实验收（2026-10-17）

用户告知额度恢复，沿用保留的独立登录，冻结 main `0d93989a172dd15800c7d72cfc3f0c72e54706e5`。额度一次采样返回 Fresh：5 小时已用 6%、周已用 54%，problem=false。文本单请求返回 200/SSE、有效完成终态，随机 nonce 核对通过。工具阶段两次网关请求均 200/SSE：caller 执行一个纯内存工具，重建 Runner 后回传结果，最终只存在于工具结果中的随机 marker 核对通过。没有重新登录、没有失败重放，CLI 内部网络次数未计量。

真实计量暴露后续修复点：工具调用段 analytics input=3109/output=32，最终段 input=6332/output=87，后端将 CLI result 整轮累计 usage 直接作为最后一段用量，可能重复已交付部分。backend owner修复同CLI turn分段计量，manual owner补合成累计/多工具/未知边界，不增加真实模型请求。当前主工作区 app/mac/main.swift 与 ShotProbe.swift 为其他任务改动，不纳入本任务。

计量契约依据复核：[Claude 官方 cost-tracking](https://code.claude.com/docs/en/agent-sdk/cost-tracking) 明确 streaming input 的 result.usage 仅当前用户轮累计；同 assistant message ID 需去重。assistant/message_start 的 output_tokens 是占位，真实分步输出计数来自 includePartialMessages 的 message_delta.usage，最终 result 为整轮累计。修复应避免占位覆盖真实值、重复事件累加和最终累计重复入账；缺失与不一致维持未知，不补零。

计量修复完成：按原生消息 ID 与可信 stream delta 建立当前 CLI turn 账本，工具响应扣除已交付部分，最终累计同样扣除；新普通输入重置账本，未知、下降、溢出与 conversation_reset 不伪造数值。`manual-subscription-composition.py --case usage` 的 8 组合成验收通过，同时检查原生 HTTP usage 和 gateway analytics：可信输出差额、占位未知、多工具去重、新轮归零、缺字段、累计低于已交付、低于已观察、重置未知。仅临时源码副本插入唯一消费完成 ack 消除 CLI stdout/MCP 调度竞态，产品不保留验收 hook。

最终 fmt、workspace all-features check、全 targets/all-features clippy -D warnings、bindings build、脚本 AST、文档相对链接与 diff 检查通过。真实文本/工具成功证据对应修复前冻结提交；计量修复由上述合成契约验收，没有再次调用真实模型。本轮仅提交 ai-provider 计量、人工脚本和相关文档，其他并行 Mac 改动由原任务管理。Claude 单订阅接入的登录、额度、真实文本和跨 Runner 工具往返已全部取得证据；未做多个真实 Claude 订阅之间的 fail-over，登录继续保留。

## Mac 设置重写（2026-10-17）

用户指出“模型路由”设置页仍是有序 fail-over 时期的早期版本，要求按当前路由与订阅聚合能力重写。此前该页只编辑名称与成员顺序，策略、权重、会话保持和提供商额度上限仅被透传；Mac 也没有 Codex 订阅认证、账户尾号、额度或披露的入口。

- 路由页改为左列表、右现状：策略与会话保持的含义；各成员的认证方式、账户尾号、登录状态、适用其模型 ID 的额度窗口（已用比例、上限、重置、过期）与路由不会使用它的原因；行内登录、双击编辑其提供商；成员含订阅时显示核心披露。编辑 sheet 增加策略、会话保持与权重（含所占比例），＋ 菜单按提供商分组并禁用其他协议的模型。新路由默认智能策略、开启会话保持（实现默认，可由用户改）。
- 提供商编辑器：“使用 Codex 订阅…”先确认披露，固定协议与地址；“保存并登录…”保存后留在编辑器并开始登录，登录 URL 自动打开浏览器。新增“额度与使用上限”页，按窗口选择上限。列表副标题区分订阅账户与登录状态。
- 披露与固定目标原只在已保存认证上可得；bindings 新增静态 `codex_subscription_target()`，由 application 的 `SubscriptionTarget::codex()` 给出协议、地址、四项组合字段（含 `CodexUsage` 额度来源）与披露，`CODEX_SUBSCRIPTION_PROTOCOL／ENDPOINT` 常量统一校验、请求与导出，Mac 不硬编码。
- 合并时 main 已改为组合选择订阅（删除 `SetCodexSubscription`，新增 Claude CLI 组合）：编辑器持有组合状态，选订阅即采用该组合并以 `Keep` 保存，“使用 API key…”回到 HTTP／Native／ApiKey／None；订阅名称按认证来源显示（Codex／Claude Code）；非 HTTP 后端不显示、不校验地址；登录按钮取 `login` 动作。新建 Claude CLI 组合与退出登录见下方补充。
- 额度只读 core 观测：视图显示时在请求队列读取，不占 `isLoading`；采样仍由 core 决定。未登录提供商不读取：核对 `read_usage` 在凭据解析处失败、不发 HTTP，但会给出泛化“额度暂不可用”，与“需要登录”重复。
- 网关运行期冷却／探测状态没有 bindings 读取入口，页面不显示“冷却中”或逐次切换原因；切换记录在“分析”窗口。是否新增该入口待需求出现再定。

验证：cargo fmt --check、cargo check --locked --workspace --all-features、clippy（-D warnings）与 application／bindings `--no-default-features` check 通过；`build-macos.sh --build-only` 零警告。临时 Python 绑定脚本在隔离 home 中验证：导出返回 Responses v1、Codex 地址与披露；订阅提供商保存为未登录 OAuth、带披露与“登录…”；错误地址被拒；API key 提供商额度 Unsupported；权重路由与 `window:18000` 80% 上限重开后保留。临时离屏渲染（仅 `.prohibited` 激活策略、隔离 home、不显示窗口）检查路由页、编辑 sheet、提供商编辑器、额度页与合成额度行，修正了脚注右对齐、权重不易辨认为可编辑和重复的未登录提示；临时代码与截图不提交。未进行真实登录、额度读取或模型请求，交由用户验收。

安装：变基到 main `7fc9b31` 并改用组合选择订阅后，提交 cf6dc9e 快进合入本地 main，在独立工作树构建并安装到 `/Applications/Velune.app`（0.1 beta.1）；安装时没有运行中的 Velune。codesign 严格校验通过，构建清单 source_commit 与该提交一致、dirty 为 false。合并后复跑临时绑定脚本：导出组合为 Http／CodexResponses／CodexOAuth／CodexUsage，以 Keep 保存的订阅提供商为未登录 OAuth 并带披露，错误地址被拒，权重路由与上限重开后保留。设置窗口实际交互、真实订阅登录与额度显示待用户验收。

补充（2026-10-17）：用户要求在 main 上补齐剩余缺口。

- 提供商编辑器新增“使用 Claude Code 订阅…”，与 Codex 共用确认披露、以 `Keep` 保存并“保存并登录…”的流程；bindings 新增静态 `claude_cli_subscription_target(binary)`，由 application 的 `SubscriptionTarget::claude_cli` 给出 Messages／空地址／`ClaudeCli` 后端／Native／`ClaudeCli` 认证／`ClaudeCliUsage` 与披露。披露文字统一归 `SubscriptionDisclosure::for_source`，已保存认证与选择前呈现同一来源。
- Claude Code 组合以可执行文件字段替代服务地址：自动填入本机发现的第一个 `claude`，可从“已发现”或“选择…”更换，保存前要求绝对路径；已有 CLI 提供商可改路径。
- 已登录且带 `logout` 动作时显示“退出登录…”，确认后调用 `authentication_logout`；只退出该提供商独立的 CLI 配置目录，运行中或登录中由 core 拒绝。
- `ORGANIZATION_SCHEMA_VERSION` 仅由 local-runtime 使用，按该 feature 编译，消除 `--no-default-features` 警告。

验证：cargo fmt --check、workspace check、clippy（-D warnings）与 application `--no-default-features` check 无警告；`build-macos.sh --build-only` 成功。临时 Python 绑定脚本在隔离 home 中验证导出组合与披露、相对路径被拒、以不存在路径保存的 CLI 提供商为未登录 OAuth 并带披露与 login／logout 动作、额度 Unknown。临时离屏渲染检查 Claude 提供商编辑器与新提供商的两个订阅入口，修正了按钮中英文间距；临时代码与截图不提交。未进行真实 CLI 登录、退出或额度读取，交由用户验收。

安装：补充提交 597a524 快进合入本地 main，在独立工作树构建并安装到 `/Applications/Velune.app`（0.1 beta.1）；安装前正常退出了运行中的 Velune。codesign 严格校验通过，构建清单 source_commit 与该提交一致、dirty 为 false。Claude Code 订阅的实际创建、CLI 登录／退出与额度显示待用户验收。

提供商模板（2026-10-17）：用户指出编辑器中逐个罗列“使用某订阅…”的入口无法随订阅增加扩展，并提出订阅锁定协议与地址，可理解为提供商模板；结论已归 PRD 与 ai-service 设计。

- application 新增 `provider_templates()` 有序目录（Codex 订阅、Claude Code 订阅、API 提供商），每项含种类、名称、简介、完整组合、本机命令与披露；删除 `SubscriptionTarget` 及 `codex_subscription_target`／`claude_cli_subscription_target` 导出（hard-cutoff）。
- Mac “＋”打开模板选择（订阅／API／从 Agent 运行时导入），选中订阅显示披露，“继续”在同一 sheet 进入编辑器；原底部“从运行时导入…”并入此选择。订阅提供商协议与地址以只读值显示，移除换用订阅的按钮、确认弹窗与换用 API key 入口；新提供商不显示“清除认证”。可执行文件命令取自模板。
- 未为订阅模板预填模型：core 尚无订阅模型发现，模型仍由用户添加或从模型模板选取。

验证：cargo fmt --check、workspace check、clippy（-D warnings）与 application／bindings `--no-default-features` check 无警告；`build-macos.sh --build-only` 成功。临时 Python 绑定脚本在隔离 home 中验证三项模板的组合与披露，Codex 模板以 Keep 保存为未登录 OAuth，Claude 模板空路径被拒、填入绝对路径后保存，API 模板以 key 保存。临时离屏渲染检查模板选择、三种新建编辑器、已有 Claude 提供商与提供商列表；临时代码与截图不提交。真实添加流程与登录交由用户验收。

安装：变基到 main `10fb7ce` 后提交 63954c8，快进本地 main（共享检出当时位于其他分支，直接快进 main 引用），在独立工作树构建并安装到 `/Applications/Velune.app`（0.1 beta.1）；安装时没有运行中的 Velune。codesign 严格校验通过，构建清单 source_commit 与该提交一致、dirty 为 false。

## Kimi Coding Plan 可行性研究（2026-10-19）

用户要求先看能否增加 Kimi Coding Plan。本轮仅调查和建议，不改产品源码、不读取凭据、不发起认证/模型/额度请求。基线 main `9c00ef8`，初始工作区干净；root负责协议与现有组合差距，codex_source负责第一方额度实现/账号边界，advisor负责最小接入范围判断。

已核对 [官方概览](https://www.kimi.com/help/kimi-code/membership-guide) 与 Context7 的官方文档索引：Coding Plan 可在控制台创建独立 API Key，OpenAI base 为 `https://api.kimi.com/coding/v1`（海外 `.ai`），支持 Chat Completions 与 Responses；Messages base 为 `https://api.kimi.com/coding/`，实际 `/coding/v1/messages`。它与 Moonshot 按量开放平台的地址/key不同，不需要依赖本机 Kimi CLI。当前 Http/Native/ApiKey 组合可作为请求路径基础，新的提供商模板只装配能力，不创建贯穿调度的 Kimi 供应商类型。是否实际兼容仍须后续受控真实验收。

[官方错误参考](https://www.kimi.com/code/docs/kimi-code/error-reference.html) 显示套餐能力不足也可能401；5小时/老套餐周/月额度耗尽及账号并发限制均可能403；429对应瞬时频控与引擎过载。现gateway通用403停止，结构化quota code仅识别三个通用值，因此不能声称填地址/key就完成套餐耗尽fail-over。需在请求适配边界归一化可验证错误事实，不能把所有403改成可切换。思考工具续接需要保留reasoning_content等上游协议字段，现best-effort转换不等同已验证兼容。

[官方社区倡议](https://www.kimi.com/help/kimi-code/community-guidelines) 允许个人交互式第三方编程工具，要求真实客户端身份，不允许伪造User-Agent、转售或非个人交互批处理。方案使用Velune真实身份，不照抄伪装Kimi CLI的社区header。额度采集可行性、窗口/账号归属及最小范围建议继续归本节，不将未确认提案写入PRD。

固定第一方实现为 [MoonshotAI/kimi-code 5710c5496a213742ca0687f69dd8eeb5bde52dbf](https://github.com/MoonshotAI/kimi-code/tree/5710c5496a213742ca0687f69dd8eeb5bde52dbf)；旧 kimi-cli 已归档，不作为当前实现基线。`packages/oauth/src/managed-usage.ts` 使用 GET `/coding/v1/usages`，Bearer + Accept JSON；payload.usages 包含可选 `limit_5h`、`limit_7d`、`limit_month_total`、`limit_month_code`（used_ratio、reset_time），boosterWallet包含余额/剩余额度及月支出限制。`managed-userinfo.ts` GET `/coding/v1/me` 有 user_id与区域信息；接口本身未给稳定subscription id。当前调用链先 ensureFresh OAuth，未取得普通 Coding API Key 对这两个接口的真实权限证据。只能把它们称作官方公开实现内部接口，不能承诺 key-only 额度/身份可用，更不能为此新增 daemon 依赖或冒用内部 OAuth 客户端身份。

[官方会员权益](https://www.kimi.com/help/kimi-code/benefits) 明确所有设备/API Key共享额度，新套餐5小时+会员月总额、旧套餐另有周窗口；不能固定假设所有订阅都存在周窗口。同账户多个Key不能当多个独立订阅；没有稳定账户证据时，key哈希不构成订阅身份。开启加油包后，额度耗尽可自动使用充值余额且不返回quota错误，因此普通模型调用成功也不能证明仍在订阅额度内；不由Velune替用户打开该选项。

建议产品范围：增加一个Kimi Coding Plan模板，复用Http/标准协议/ApiKey，一订阅一个provider；主动额度来源在确认key权限后装配，失败保Unknown/Unsupported。完整聚合接入同时补服务边界错误语义，向gateway输出中立分类，不把品牌贯穿调度，不把所有403都改成可切换，不把401套餐权限误报成Key失效。优先原生上游协议完成文本/思考工具往返、计量和额度边界，再验证已有异协议转换。当前可行性结论与范围建议已形成，未进入源码实施、未进行真实调用。

2026-10-20 用户认可Kimi模板/额度/错误语义方向，并授权以指定Coding Key验证第一方查询接口、继续收敛；权限变更已归PRD与sources。实际私有文件位于所指项目的隐藏secrets目录，仅程序内提取专用Coding key/base，未读取其他变量值或保存响应。正常TLS、无重定向、真实Velune身份，共两次GET，无模型/登录/重试。`/coding/v1/usages` 与 `/coding/v1/me` 均HTTP200；前者实际返回5h/week数值比例及reset字段，月窗口/wallet缺失；后者有user_id/global_id，仅输出存在性。由此否定“普通Coding Key必然不能查额度”的假设，但不推广为所有套餐/区域都支持。

技术收敛、脱敏证据、验收判据与线性顺序已合并到 [Kimi Coding Plan接入收敛](kimi-coding-plan.md)。建议复用request profile新增KimiCoding而非另增ErrorPolicy配置轴，KimiUsage独立来源，Messages为首版原生上游。凭据generation控制旧任务，经过/me验证的owner控制订阅归属；同owner换key不清阈值/冷却，不按keyhash复制订阅。当前阶段结果已形成，产品源码未改、未发模型请求；后续实施及推理验收仍为待完成事项。

## Kimi Coding Plan 实施（2026-10-20）

用户明确开始实施。已更新PRD、AI service契约及 [Kimi收敛/实施记录](kimi-coding-plan.md)。main ea7362b初始干净；主执行者负责整合/文档/最终验证，transport负责ai/gateway，source负责app/API/bindings，platform负责最小平台消费者，manual负责回环合成验收。无真实模型/凭据读取，复用此前两个HTTP200证据；不新增自动测试/CI，不push。Mac修改后按既有要求构建安装，Android不增不存在的配置入口。

接入已实现：Kimi模板组合固定HTTP/Messages/API key/KimiCoding/KimiUsage；账户验证和去重、主动额度、使用上限及服务错误语义接入既有调度。三HTTP协议改用共享安全HttpClient。Mac既有编辑器支持订阅API key，Windows既有表单增加core模板选择；Android为新增枚举重建JNI/Kotlin/APK并保数据覆盖安装。Kimi 44组、Codex24组、Claude取消3组及原生Messages8次HTTP的合成人工证据通过；Rust静态/构建、Mac完整构建和Android build/lint通过，详细边界与安装证据归上述Kimi记录。真实推理、工具往返及海外区域尚未验收。

交付提交main `1444d56`，已从干净提交重建并安装Mac `0.1 beta.1`；manifest对应实现提交且dirty=false，配置schema仍10，安装二进制哈希与产物匹配。Android同步版本已保数据安装，Windows只取得Mac宿主静态证据。未push/PR，未进行真实模型调用；后续真实验收须另明确调用范围。
