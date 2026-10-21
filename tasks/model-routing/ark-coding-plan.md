# ARK Coding Plan：接入研究与实施

2026-10-20，用户先要求“还看看 ARK Coding Plan”，完成公开资料研究后，确认提供商内部独立额度认证并授权实施。主任务见 [模型路由 packet](packet.md)。本轮不读取真实认证或相邻项目的 Key，不启动 CLI 登录，不调用真实额度或模型端点。

## 已核对的接入事实

| 能力 | 证据与结论 |
| --- | --- |
| 推理协议 | 官方支持 Responses、Chat Completions 和 Anthropic 兼容接口。前两者 base 为 `https://ark.cn-beijing.volces.com/api/coding/v3`，Messages base 为 `https://ark.cn-beijing.volces.com/api/coding`；现有 adapter 分别追加 `/responses`、`/chat/completions`、`/v1/messages`。这是文档与本地 URL 构造的匹配，尚未真实调用 |
| 模型认证 | 数据面使用 ARK API Key，不要求通过 Ark CLI 执行模型。个人 Coding Plan 与普通平台可使用同一普通 Key 池，不能从 Key 外形判断消费哪个套餐 |
| 消费边界 | Coding Plan、Agent Plan 和普通按量 API 是不同服务目标。Coding 额度耗尽等待刷新；官方说明不继续消耗其他资源包或账户余额。配置成 `/api/v3` 则可能产生按量费用，不能作为自动 fallback |
| 模型选择 | 支持明确 model name，也支持 `ark-code-latest` 由控制台选择，后者还可能是 Auto。该 alias 不证明每次是同一个底层模型；不同模型消耗额度的系数不同，不用请求次数或本地 token 反推套餐比例 |
| 额度读取 | 官方 Ark CLI 的 `usage plan --product coding-plan` 对应 `GetCodingPlanUsage`，输出 session/weekly/monthly 的已用百分比及 reset。Coding 返回百分比，不保证存在绝对已用量和总额；月窗口按实际 reset，不能硬编码 30 天 |
| 使用场景 | 官方将个人版限定在个人 AI 编程工具场景，非编程工具使用有停用或封禁风险。Velune 经支持的 Harness 接入是否被供应商接受尚未实测，不伪造客户端身份 |

官方来源：[套餐概览](https://docs.volcengine.com/docs/ark/coding-plan-personal-plan-overview?lang=zh)（页面更新 2026-10-19）、[Codex 接入](https://docs.volcengine.com/docs/ark/coding-plan-personal-ai-codex?lang=en)、[OpenCode 接入](https://docs.volcengine.com/docs/ark/coding-plan-personal-ai-opencode?lang=en)、[快速开始](https://docs.volcengine.com/docs/ark/coding-plan-personal-get-started?lang=zh)。未采用火山域名下自动生成的推广文章作为接口依据。

## 额度认证与账户绑定仍有缺口

官方 [ark-cli](https://github.com/volcengine/ark-cli/tree/dd4c0dc2990da62be9fc040f431050aa2b2d508c) 固定于 `dd4c0dc2990da62be9fc040f431050aa2b2d508c`。该公开仓库主要是 README、Skills 与插件声明，没有 CLI 核心实现；不能把其文档中的代码解释当作本轮源码实证。

- [额度命令说明](https://github.com/volcengine/ark-cli/blob/dd4c0dc2990da62be9fc040f431050aa2b2d508c/skills/arkcli-usage/references/arkcli-usage-plan.md) 有 APIKey viewer 的描述。
- [认证边界说明](https://github.com/volcengine/ark-cli/blob/dd4c0dc2990da62be9fc040f431050aa2b2d508c/skills/arkcli-auth/references/auth-modes.md) 却明确把控制面划为 SSO 派生 STS 或 AK/SK 的 V4 签名，API Key 归模型数据面。APIKey viewer 不能证明仅凭普通模型 Key 可查询套餐。
- [登录说明](https://github.com/volcengine/ark-cli/blob/dd4c0dc2990da62be9fc040f431050aa2b2d508c/skills/arkcli-auth/references/arkcli-auth-login.md) 显示 SSO 会派生 profile、获取对应 Key，交互流程还可能询问创建或轮换 Key。不能把引入 CLI 登录理解为无副作用的额度查询。

Magpie 固定于 `2374ffe161439b02a8e7f23b793461b39a635bf8`，其 [volcengine_usage.go](https://github.com/yetone/magpie/blob/2374ffe161439b02a8e7f23b793461b39a635bf8/internal/provider/volcengine_usage.go) 另外保存 AccessKeyID/SecretAccessKey，以 HMAC-SHA256 V4 签名调用 `open.volcengineapi.com` 的 `GetCodingPlanUsage`；Coding Plan 与 Agent Plan 使用不同 action。该实现的卡片按 AK 与 action 去重，不足以证明模型 Key 和查询账户一致，也不保证同账户两把 AK 会合并。其 [子系统说明](https://github.com/yetone/magpie/blob/2374ffe161439b02a8e7f23b793461b39a635bf8/docs/subsystems/providers-accounts.md) 明言尚无真实账户响应实证；本轮也未替它做真实验证。

完整聚合必须分别回答两件事：查询凭据能看到哪个账户/席位的额度，以及实际模型 Key 是否属于该额度范围。个人版 owner、IAM 用户和企业席位不能混为一个 ID；AK 或 Key 的哈希也不能充当订阅身份。额外认证只用于固定的查询目标，不得混入模型凭据或转发给 Harness。

## 对现有 Velune 的影响

可沿用现有 HTTP 后端、三协议 adapter、安全 HttpClient、路由策略、阈值、冷却和粘性。候选组合仍是一订阅一个提供商实例，不新增贯穿系统的 ARK 提供商类别。新增模板及服务 request profile 可负责固定 Coding 目标、真实客户端身份和服务错误事实；gateway 继续只消费中立事实。

主动额度的真实缺口在 application：目前 Kimi 使用同一模型 Key 读取元数据，Claude 使用同一 CLI 登录目录；没有为一个提供商分别配置模型认证与额度认证的契约。若 ARK 必须另用 AK/SK 或 SSO，就需要提供商内独立的额度认证绑定，并将两种认证的变更版本都纳入旧采样、归属及调度失效处理。不能只向 quota_source 加一个枚举便宣称完成。

官方 [错误码参考](https://docs.volcengine.com/docs/ark/error-codes?lang=en)（更新 2026-10-17）中，`InvalidSubscription` 是 400、`UnsupportedModel` 是 404；这些应与认证失效、短暂不可用分开。429 同时涵盖套餐窗口耗尽、RPM/TPM、过载和排队限制，甚至 `QuotaExceeded` 也有多种含义。需要结合固定 Coding 目标、结构化错误和明确的窗口事实分类；不能把每个 429 都标为订阅额度耗尽。原状态和正文、取消与交付后不切换的边界继续保持。

## 实施前的研究建议与待判别项

先将范围限定为国内个人 Coding Plan，保留三协议原生能力，默认模板协议可按官方推荐采用 Responses。Agent Plan、企业席位与海外 BytePlus 是分别有目标/身份差异的后续范围，不借相同品牌混入本次。

本轮查过官方 API 文档入口与 Go SDK 的 Ark 服务目录，没有取得 `GetCodingPlanUsage` 的完整公开鉴权/身份返回契约；目录有 19 项及临时 API Key 接口，该有界检查不证明所有 SDK 或控制面都不支持。下一项判别是取得第一方对额度认证及模型 Key 归属的明确契约；仍不足时，才用经用户单独授权的普通模型 Key 对固定目标做只读权限验证，失败即记录，不改用浏览器 Cookie 或扩大为 AK/SK/SSO。若 Key-only 成立，接入可接近 Kimi；若不成立，再确认是否接受提供商内额外的只读查询认证，并先解决模型 Key 与查询账户的绑定证据。

本轮没有真实权限结论，不能断言普通 Key 一定能查或一定不能查。模板与反应式错误路由可形成较小首版，但若没有归属及主动额度证据，必须明确它尚未达到已确认的完整智能聚合验收效果。

研究阶段 advisor 建议先实现原生请求与被动 fail-over，独立确认主动额度范围。用户随后选择提供商内独立额度认证，实施方案包含主动额度，并以用户明确关联区分于自动归属核验。模型调用不依赖 Ark CLI，AK 哈希不充当订阅身份。

## 当前实施

2026-10-20 用户授权实现独立额度认证。实现同时交付模板、固定 Coding 请求适配、V4 查询、窗口及绑定生命周期、公开 API/绑定、最小平台配置与手动合成验收。保留 schema 10，新增私有字段缺省为空，不清除既有配置。模型 Key/额度凭据的同订阅关联由保存动作明确确认，不显示自动核验；变更后重新确认。未知或未确认额度只影响有上限的准入，不阻止无上限模型调用。

实现已完成。查询认证编辑是 `save_provider` 的可选参数 `quota_authentication_edit`（2026-10-21 从 `ProviderDraft` 移出，与模型认证编辑并列），与模型认证同次原子保存；SetAccessKey 保存完整 AK/SK 和本次确认，ConfirmBinding 关联当前模型认证，Clear 单独清除查询认证，None 保留。公开 `quota_authentication` 只有 configured 和 binding_confirmed，私有结构及编辑动作的 Debug 隐去凭据。无查询认证不发控制面请求，也不会借模型 Key 查询。

额度使用固定 `GetCodingPlanUsage`、cn-beijing/ark 的 V4 签名，禁止重定向和自动重试，响应正文上限 256 KiB；200 中的业务错误也算读取失败。session/weekly/monthly 用实际 Percent 和秒级 ResetTimestamp，未知窗口省略、非法或重复窗口拒绝、不伪造月长度或绝对额度。关联确认随模型/查询版本与目标失效；旧版本及已删除实例的采样不能发布。查询凭据变更失效旧 Runner，但保留模型冷却和粘性。

共同额度生命周期另修正两处：刷新失败会让旧观测过期；采集中收到更晚的耗尽事件时，丢弃该次旧采样并按需重取，不能把耗尽前观察重新发布为新鲜值。

## 验收结果

| 范围 | 实际结果 |
| --- | --- |
| ARK 合成 HTTP/application | `manual-ark-coding.py` 37 组通过：模板、三种原生协议、Python 独立复算 V4 签名、认证隔离、用户关联准入、cap、换模型/查询凭据、旧任务失效、耗尽后重取、原错误逐字节保真、窗口与业务错误边界、重复凭据和按量地址拒绝、清除/保存重开、缺省字段不重置既有配置 |
| Kimi 回归 | `manual-kimi-coding.py` 44 组通过，含身份去重、cap/cooldown/sticky、旧任务与读取失败 |
| Codex 回归 | `manual-codex-subscriptions.py` 24 组通过；机械补充新的可选编辑字段、profile 映射与限定范围的临时目标重写 |
| Rust | 全工作区 all-features check、全 targets/all-features clippy -D warnings、fmt 和 application/bindings 无默认 feature 检查通过；最后额度采集收口后再次检查受影响 application |
| 平台 | Mac 完整生成 Swift 绑定/SwiftPM 构建及签名校验通过；Android 完整 JNI/Kotlin/APK 构建、lint 与保数据覆盖安装通过；Windows 源码取得 Mac 宿主静态证据，未做 Windows 原生运行 |

所有行为请求只发往回环夹具，生产源码没有测试目标开关，也没有新增自动化测试或 CI。真实 AK/SK 权限、ARK 返回格式与文本/工具、真实耗尽尚未验收；本版关联由用户声明维护，不能检测不同 AK 下是否重复同一订阅。

Android APK 已保数据覆盖安装，SHA-256 为 `46604acfa94737bd6f143ecce1a49110d8c0bc2d0d12e0bfc7b11741a222d53a`。

## 本地交付

实现提交 main `7adc6505122bd48d22eae1872aeb62058d6dc0b4`。从干净提交执行完整 Mac 构建，正常退出运行中的 Velune 后安装 `0.1 beta.1`。安装包的 build-manifest 对应该实现提交、dirty=false、配置 schema10；安装后 codesign 严格校验通过，以下 SHA-256 与构建产物一致：

- Mac 执行文件：`1c1954a2946b1cefbeec4c670cc1fbb3c745f81628e24d7f9ba41c02cd69cb43`。
- Rust 动态库：`7196d586411d094c73856e8f5501000ad90f86793c1de425113771fa9763ed30`。

没有启动真实账户操作、推理或额度读取；未 push、PR 或部署。Windows 仅有宿主静态检查，实际配置交互和真实订阅验收仍待用户进行。
