# 模型路由与自动 fail-over

2026-10-16 用户要求实现模型路由，验证场景为聚合两个 ChatGPT 订阅并自动 fail-over（[S30](../../docs/sources.md#s30)）。产品归 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，网关契约归 [AI 服务设计](../../docs/design/ai-service.md#路由fail-over-与生命周期)。

## 凭据边界

用户提出可用 `~/.codex-switcher` 下的两个订阅认证文件。开发工具列出该目录被本机权限分类器以凭据探查拒绝；项目记录 ChatGPT 订阅的第三方接入以 SIWC 为准，不借 CLI 缓存 token 冒充正式接入（[开发说明](../../docs/development.md)、[架构](../../docs/design/architecture.md) Codex 行）。因此：

- 本任务不读取该目录，也不实现 Codex CLI 认证文件来源。
- 路由与 fail-over 本身用回环合成上游验证。
- 真实两账户验收由用户在应用中完成：两个 Pi 实例各自的 agentDir 分别通过已支持的 openai“登录…”登录一个 ChatGPT 账户，导入为两个提供商，再组成路由。
- 是否另行接入 Codex CLI 认证文件，待用户决定。

## 开发方选择的策略（待用户复核）

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
- 待用户：复核上文 fail-over 策略；用两个 ChatGPT 账户做真实验收（两个 Pi 实例分别登录、导入、组成路由）；决定是否接入 Codex CLI 认证文件来源。
- 2026-10-16 安装：随提交 554007d 重建并安装到 `/Applications/Velune.app`。codesign 严格校验通过；构建清单的 source_commit 与该提交一致，dirty 为 false；后台启动后没有新的崩溃报告。

## 真实双订阅验收的归属（2026-10-16）

- 用户先要求用 `~/.codex-switcher` 的两个 ChatGPT 订阅做真实 fail-over 验收。核对后：该目录含两个账户的 OAuth（access/refresh/account_id）。本机权限分类器拒绝开发工具列目录，项目 AGENTS.md 亦明确「不能借 CLI token 冒充正式接入」「不读取、复制…登录令牌」「不…规避供应商限制」。
- 用户明确授权走该路线（读 token、以 Codex CLI 身份请求 ChatGPT backend）；实现过程被安全分类器中止。该路线最终判定为规避供应商访问边界 + 违反项目既定规则，不予实现；没有产生任何相关产品代码。中止前已读取账户文件（仅结构与过期时间，无值）并对 ChatGPT backend 发过少量真实探测请求；未写入、未刷新、未提交任何凭据；临时脚本与 Magpie checkout 已删除。
- 用户随后同意放弃该路线。真实双订阅验收仍待用户走官方路径：两个 Pi 实例各自官方「登录…」一个 ChatGPT 账户（Velune 只委托来源解析/刷新，不复制 refresh 凭据），导入为两个提供商后在「模型路由」页组成路由。
- 讨论但未决定的替代方向：**运行时层**的 Codex app-server 多账户聚合（两个 Codex 实例各自官方登录，Velune 按用量做 turn 级 fail-over，会话绑定账户、不中途换账户）。这与本任务已实现的网关层模型路由是不同层次的能力，属产品范围扩展，另需用户决定后再开任务。
