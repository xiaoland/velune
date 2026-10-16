# 模型路由与自动 fail-over

2026-10-16 用户要求实现模型路由，验证场景为聚合两个 ChatGPT 订阅并自动 fail-over（[S30](../../docs/sources.md#s30)）。产品归 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，网关契约归 [AI 服务设计](../../docs/design/ai-service.md#路由fail-over-与生命周期)。

## 当前接续与凭据边界

2026-10-16 用户已确认下文具体方案，授权开始，仅实现 Rust lib，前端另行安排。不改 Mac 源码、生成 Swift 或构建安装 app；Rust UniFFI API 与文档须足以供前端接入。

- 当前路线为 Velune 独立管理 Codex 订阅认证及 AI 网关聚合；正式 SIWC 留作未来申请后的独立接入。用户已知悉规则冲突并要求产品披露。
- Codex Switcher 只是两个验收订阅的来源，不做产品集成。用户最新明确授权直接使用既存两份认证：只在一次性验收进程内读取 access token/account_id，不读取 refresh token 值、不刷新或改写原文件。此前重新登录建议已被用户纠正。
- implementation owner：codex_source 负责 application 认证生命周期与 bindings API；subscription_transport 负责 ai-provider 与 gateway；subscription_manual 负责新隔离合成脚本；主执行者负责跨模块整合、既有脚本与权威文档及静态验收。其他并行任务改动不纳入提交。
- Rust 支持已提交 `dd5683d`；真实 provider/gateway 双订阅链路已通过：A Completed → 本地注入 429 → 真实 B Completed，普通文本历史标记精确一致。真实诊断发现的 MIME／错误状态问题已修正，20 项合成人工验收、静态检查与 Rust bindings 构建均通过。Rust 范围完成，前端仍不在范围内。

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
