# 输入与执行链复核

## 现行能力与责任

- `ComposerDrafts` 管窗口内编辑模型和导航指针；`ComposerInputClient/Worker` 管共享目录、原修订基线与附件缓存。正文／附件按 InputID，配置按目标 ContextKey，CAS 彼此独立。跨 group 移焦点只换目标，同 group 换 Tab 才恢复另一份输入。
- `ComposerContext` 从已加载目标解析菜单、额度与实际提交配置；目标尚未加载时不从旧画面播种选择。侧聊保留自己的临时 Store 配置，不写普通目标目录。
- `ConversationTab` 捕获正文、附件及清空后的编辑修订；发送／排队／steer 失败仅恢复对应输入，不覆盖之后的编辑。首次创建先接住原生身份，再派发首条消息；全局新建输入另换空项。
- `OutgoingSubmissions` 提供发送／排队／steer 即时占位，以 application 稳定身份接续原生回显，队列受理后转交会话权威队列；同一个回显只认领一次。
- 核心 `TurnRequest` 保存排队当时的模型、推理与权限。`ExecutionConfiguration` 捕获实际已开始执行的 runtime、gateway 和 backend；配置保存使后续准备失效重建，不中断已运行请求。现有合成 429／改配置实测可继续复用。
- Pi 继续由核心声明为 `continue` 文本；原生继续、队列暂停与自动重试分别有自己的准入。打开历史优先于需启动原生运行时的模型目录，不通过增加 Transport 并发隐藏排队问题。

## 已确认并纠正

1. **P1：会话打开失败后输入／目标回滚不完整。** `selectConversation` 已发出新目标的 composer 导航并清 `isDraft`，失败仅还原选择；可能显示 A 的历史却保留 B 的输入，或把尚未首发的草稿当成既有会话发送。现在还原原草稿身份，并发同一 composer 导航事件恢复原输入。原有 enqueue generation 仍拒绝过期结果，deferredSelection 继续执行后来的选择。实际窗口模型的失败 Transport 窄检查覆盖既有／未首发来源；导航 owner 独立复核并补查找回包及延迟目录边界。
2. **P2：原生创建后的首发失败重新占用全局新建输入。** `ConversationTab.send.onFailure` 无条件归还被消费 InputID。现在共享输入恢复入口明确接受本次创建的来源 Store，只有该来源仍未取得原生身份才归还。不能按 InputID 的任意历史创建记录禁止恢复，因为同一内容可用于后续新会话。已扩展原生创建检查，区分创建前失败、后续编辑优先、创建后失败留在原会话及旧 InputID 再次创建。
3. **P2：返回旧运行时时只恢复模型，丢失最近推理／权限。** `resolve_next_turn` 与核心 `requested_*` 仅查全会话最后一轮；A→B→A 时 A 的最近配置被当成未知。新增共享 `latest_runtime_turn_settings`，菜单投影和真实请求共用同一规则；最新 A 轮中的未知项仍未知，不逐字段复活更旧值。模型选择有自己的别名／目录逻辑，未强行合并。
4. **文档规则分叉。** S78 已规定运行时选择按目标持久化，但 `composer-choices.md` 仍保留离开就丢弃临时运行时的旧句；Mac README 也仍把执行选择归于 InputID、把所有首发失败归还全局新建项。已按现行产品决定和上述恢复边界精确修正三处，没有据旧文字反改正确实现。

这些是状态生命周期没有一起收尾的缺口，修复集中在已有责任处，没有增加新的状态管理层。内容／目标拆分、执行配置快照和统一提交生命周期本身有明确的不同寿命及真实用例，不能仅因文件或字段增加判为架构腐坏。

## 证据与使用边界

- `velune-application-api` release `next_turn::tests` 窄测通过：9 passed、0 failed，包括两项新增返回运行时／未知项回归。使用显式 `~/.cargo/bin/cargo`、`CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo` 并取消两项 Rust wrapper。PATH 的 cargo shim 会重新注入 wrapper，首次仅终止自己的构建树后改用上述入口；不把该环境故障计为产品测试失败。
- Swift parse、rustfmt 及本组 `git diff --check` 已通过；本组改动文档的 31 个相对链接／标题锚点检查通过。最终签名产物的 `--check-new-tab` 与 navigation-only 均 exit 0。前者实际执行创建前失败、后续编辑保护、创建后不归还全局输入、旧 InputID 再次创建的断言；后者验证打开失败后输入／未首发身份恢复及 deferred 导航。原始结果见本轮验证目录 `native-new-tab.log` 与 `native-navigation.log`。
- 既有任务包中的跨端构建、真实合成 Pi／Codex 回环、CAS 与附件同步证据仍适用于未改路径；不重复整个运行时／视觉矩阵。
- `OutgoingSubmissions` 目前没有端到端提交 operation ID，只认新稳定 User 身份。AgentSender 已排除且普通外部写入受 native 控制权限制；未证明可达的误认场景。本轮保留此限制，不用正文猜测替代身份（附件与交接会改写发送正文）。
- 不读取真实会话、凭据，不执行真实迁移、不重装、不提交。跨独立执行宿主的共同 composer 保存端设置仍为既有待部署边界；网络故障根因仍开放。
