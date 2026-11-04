# AI 服务与 LLM Gateway

本页维护 AI 服务、提供商与 LLM 网关的职责划分与跨 unit 契约。产品行为见 [AI 提供商](../prd/ai-providers.md) 与 [模型路由](../prd/model-routing.md)；各 unit 的接口见 [gateway](../../packages/gateway/README.md)、[ai-provider](../../packages/ai-provider/README.md)、[application](../../packages/application/README.md) 与 [bindings](../../packages/bindings/README.md)。

按需阅读：[提供商配置](#提供商模型与模板) · [认证](#认证) · [执行后端与额度](#能力组合与执行后端) · [导入](#运行时提供商导入) · [网关数据面](ai-gateway.md) · [组织账单](ai-billing.md)。

## 职责

AI 服务是独立 lib，领域不限于 LLM；AI 服务与 Agent 运行时互相独立。gateway 是 LLM Gateway，是 AI 服务的一种应用模式：承担同协议原生透传、异协议 best-effort 转换、模型路由与 fail-over。Harness 是它的一类调用方，网关不是 Harness 专属；应用直接消费 AI 能力时不经过网关，AI 契约也不反向依赖 gateway。非 LLM 能力按具体用例扩展，不预建其他网关或空领域框架。

| unit | 负责 | 不承担 |
| --- | --- | --- |
| `ai` | 各操作的调用方与 provider 契约、原生内容封装、错误与生命周期语义 | HTTP、配置存储、Harness 规则；以 LLM 消息或 token 定义所有操作 |
| `ai-provider` | 对确定目标执行一次协议操作：认证应用、HTTP、原生响应与终态观察；请求范围的协议转换 | 路由选择、fail-over、隐藏重试 |
| `gateway` | 入口访问控制、模型解析、路由快照、请求约束、转换装配、候选选择、冷却与 fail-over、下游响应提交、用量采集 | 会话或工具执行、Harness 配置解释、读取应用目录 |
| `application` | 配置仓库与私有认证、认证生命周期、额度采集、跨域装配、导入与分析存储 | 再造协议 decoder，或要求 UI 补偿业务约束 |
| `agent-runtime` | Harness 原生配置、身份、模型能力与来源兼容设置的投影 | 网关路由权；把 Harness 参数变成 AI 服务的通用参数 |

领域包不读取全局环境、配置或认证文件，也不依赖 UniFFI。

## 提供商、模型与模板

提供商持有协议、服务地址与认证，拥有多个实际可调用的模型条目；模型条目保存稳定的 `recordKey`、精确的 `providerModelId`、显示字段、可选能力与逐请求合并进上游请求体的额外请求参数（`requestParameters`，冲突时覆盖适配器所设字段，上游不识别的字段以其自身错误失败；速度变体等服务档位由此声明为独立条目）。application 为新条目生成记录键，用户修改 API ID 时运行时选择仍引用原记录。网关派生 `velune/model/<recordKey>` 作为调用入口，不接受裸记录键作为 wire model；直接模型调用不需要另建路由。导入按来源提供商建立条目，不按名称合并跨提供商记录。

模板只复制名称、建议 API model ID、图标与可选能力，创建后成为独立快照；不复制认证、服务地址、记录键或 adapter 投影，调用链不保留模板身份。提供商模板（`provider_templates()`）是有序目录，每项含完整的能力组合、订阅或 API 种类、需要定位的本机命令与选择前披露，平台据此呈现“添加提供商”，不自行罗列组合。公开模型目录（models.dev）的适配归 application 的模板用例：显式拉取、保留来源提供商与模型 ID，能力只作建议，缺失或为零的规格保持未知；拉取不消费认证、不创建提供商，也不在后台更新已保存的快照。

未知能力保持未知，空推理等级列表表示明确不支持；不全局强制输出上限或统一 effort。缺少描述性规格不阻止原生调用，某个 Harness 必需的字段只在对应 adapter 的准备边界检查。

配置只接受 schema 11，切换版本前必须按[手动迁移约定](../development/migrations.md)将已有配置转为当前格式。现有加载器仍会把未迁移的旧普通配置原子重置为空配置，不转换字段、不保留旧文件；这是当前实现行为，不能代替升级前的手动迁移。未来 schema、损坏 JSON 与当前版本的未知字段明确拒绝。原 Harness 文件、会话与已有平台秘密不在重置范围内。

## 认证

application 私有保存 API key 或认证来源定位，并向 gateway 注入中立的异步解析器；gateway 捕获目标并解析本次认证，不接收 Harness 类型、路径、来源 JSON 或 helper CLI；ai-provider 只消费本次解析出的短生命周期认证。

提供商字段与认证编辑一次原子提交。认证编辑区分保留、设置新值与清除，取消草稿或读取失败不清空既有凭据。私有文件以 0600 创建；列表、snapshot、Debug 与日志不含秘密，显式单条读取才返回。公开摘要只描述认证类型、配置状态、来源与可执行动作。

- **来源型 OAuth**：保留原来源的登录与刷新，由 adapter 在原存储锁内刷新，不复制 refresh credential。凭据 helper 在 Unix 上以独立进程组、在 Windows 上以 kill-on-close Job Object 运行，超时、取消与网关关闭时整树终止。
- **Codex 订阅**：在 Velune 中按账户独立登录与续期，复用 Codex OAuth 客户端身份与 ChatGPT Codex backend 约定，不是正式 SIWC；不导入 `auth.json`。多订阅轮换与 SIWC 的规则存在冲突，产品按用户要求在接入与路由处披露，不宣称供应商认可（[PRD](../prd/model-routing.md)）。
- **Claude 订阅**：经本机 Claude Code 执行后端接入，登录、秘密与续期归 Claude Code。独立登录使用专属 `CLAUDE_CONFIG_DIR`；也可沿用本机默认登录（不设置该变量）或某个 Claude Code 运行时实例的登录（使用其 `agentDir`），沿用时只经 `claude auth status` 检查状态。子进程保留真实系统 HOME 以访问 macOS Keychain。同一 Claude 账户只能绑定一个提供商。

## 能力组合与执行后端

提供商实例组合协议、执行后端、请求适配、认证来源与额度来源；有效组合在保存时校验，品牌只作为模板装配已支持的组合，不贯穿调度。一个订阅对应一个提供商实例，认证与额度观察关联同一绑定版本，换号使旧观察、执行会话与粘性失效。不提供任意脚本、任意凭据目标或通用插件框架。

application 持有认证生命周期、额度采集与跨 Runner 的执行后端句柄；gateway 持有候选选择、使用上限、冷却与 fail-over；ai-provider 处理原生协议、请求适配与实际执行。HTTP 认证恢复由具名恢复能力与稳定身份判定，不按品牌判断。

- **HTTP**：三种协议共用由 builder 构造的 HttpClient，禁用重定向与底层自动重试；代理、TLS 与超时仍可配置。ai-provider 输出中立的失败事实（认证、权限、配额耗尽、短暂限流等）并保留原 HTTP 状态。
- **Claude Code 后端**：只暴露调用方声明的工具，工具由调用方执行。执行会话按提供商绑定与显式会话身份隔离。尚未完成的工具交互绑定原提供商；取消、换绑或进程失败后，迟到的工具结果在候选选择前被拒绝，不启动新会话重做。外层 Harness 结束时显式结束该轮，正常完成的空闲进程可以复用。
- **Kimi Coding Plan**：HTTP、Messages、API Key、KimiCoding 请求适配与 KimiUsage 额度来源。账户由服务区域与 `user_id` 标识，经固定 `/me` 验证；同一区域内一个账户只能归属一个提供商。凭据版本取消旧任务并阻止旧结果回写，同账户换 Key 保留使用上限与冷却。
- **ARK Coding Plan**：额度认证（AK/SK）由提供商单独持有，只供 application 的固定控制面调用，不进入模型认证或 Harness。用户保存时确认它与当前模型认证属于同一套餐；这是用户确认的关联，不是自动核验，更换任一认证或目标都撤销关联。
- **账户额度来源**：`ApiAccount` 只接受以 API key 直连 DeepSeek、Moonshot、OpenRouter 精确官方主机的组合，查询发往同一主机，不跟随重定向、不重试，响应限 256 KiB。

额度窗口按实际返回解析，缺失保持未知，不补零、不猜套餐类型。具体窗口 ID 与字段见 [application](../../packages/application/README.md)。

## 组织用量与账单

见[组织用量与账单](ai-billing.md#组织用量与账单)。

## 路由与 fail-over

见[路由与 fail-over](ai-gateway.md#路由与-fail-over)。

## 原生派发与观测

见[原生派发与观测](ai-gateway.md#原生派发与观测)。

## 运行时装配

application 优先选择运行时支持的同协议入口，否则选择运行时的原生入口并由网关转换；把所选提供商模型与中立的网关接入信息交给 adapter，执行侧只获得 loopback 入口与临时 token。额外的 wire 模型字符串以显式 alias 映射到稳定的模型记录，不按同名推断目标。会话的只读历史、resume、审批与取消都不进入网关。

提供商配置中的模型映射可以携带 `piProjection`，其类型与转换归 Pi adapter，不是通用模型能力；它保留来源 Pi 的兼容设置、采样参数与按推理级别的参数，使替换 provider 或 URL 不改变 SDK 编码。普通编辑保持协议、记录键与 API model ID 时保留投影，改变这些关联时清除。`ai`、`ai-provider`、公开模型 DTO、UniFFI 与平台 UI 都不携带 adapter 私有元数据。

## 运行时提供商导入

导入覆盖来源中的提供商、协议、端点、有效模型、能力参数与认证来源。application 在预览与应用时按所选运行时实例重新解析目录与执行配置，不接受调用方覆盖路径；来源的非秘密规范化快照变化使旧预览失效。预览不执行凭据命令、不刷新认证、不访问模型服务，也不返回秘密；无法由当前网关保持语义的配置显示原因。

| 来源 | 读取 | 导入为 |
| --- | --- | --- |
| Pi 1.0 | 有效模型配置与认证 | HTTP 提供商；静态 API key 复制到私有配置，OAuth 保留来源引用 |
| Codex | `config.toml` 的自定义 `model_providers` | HTTP 提供商，只复制字面 key |
| Claude Code | `settings.json` 的 `env` API 与登录 | HTTP 提供商，或沿用该运行时登录的 Claude CLI 提供商 |

`env_key` 与 `apiKeyHelper` 不解析，导入后在 Velune 填写；Codex 的 OpenAI／ChatGPT 登录与 `auth.json` 不读取。导入只支持默认本机执行的实例；DSH 与通用 ACP 没有导入。来源以 Bearer 发送 Messages key 时在预览中提示。

导入在当时形成快照，不做双向同步。重复项默认跳过，替换须明确选择：替换更新参数与认证、移除未选模型、保留仍被选中模型的记录键。应用前重新核对来源与目标配置，变化后要求重新预览；派发时核对保存的来源执行绑定，避免来源端点改变后把凭据发往另一个目标。导入不自动改变当前会话模型。

## 协议转换

见[协议转换](ai-gateway.md#协议转换)。

## 用量与性能分析契约

见[用量与性能分析契约](ai-gateway.md#用量与性能分析契约)。
