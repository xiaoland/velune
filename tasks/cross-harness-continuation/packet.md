# 跨 Harness 会话接续

2026-10-06 用户接受“目标原生会话＋上下文交接＋只持久化关联元数据”，授权从上一轮三状态分离继续实现。产品边界归 [PRD](../../docs/prd/index.md#会话内容展示)，跨单元方案归 [架构](../../docs/design/architecture.md#历史来源下一轮选择与实际执行)。此前状态及安装证据见 [配置与协议任务](../settings-protocol-refinement/packet.md)。

保留原生会话，在目标运行时创建原生会话，由 adapter 交接可携带上下文，不重放工具执行或审批状态。Velune 只保存原生引用和切换位置，不保存消息副本；重启后通过原生来源重新投影同一逻辑会话。下一轮意图、历史来源与当前执行者保持独立。

所有权：ai_service_audit 继续负责 conversation、agent-runtime、application、bindings 及必要 helper 的 Rust 贯通和隔离原生验收；session_management 继续负责 Mac 源码及实际 AppStore 手工端到端脚本；root 负责权威文档、任务协调、集成静态检查、构建和安装。handoff_advisor 只读解决逻辑身份、切换边界、失败恢复与管理语义的关键判断，不作为 reviewer。owners 不回滚他人改动。仓库引用的 guides/delegation.md 仍不存在，按显式稳定 owner 协调。

公开接口的待核对事实：Pi 1.0.2 可初始化原生消息；Codex 0.159.3 的 history 导入公开标记不稳定，不用作产品依赖；DSH 0.2.0-rc.2 ACP 未暴露 history seed。候选共同路径是在目标原生会话的首次用户请求中交接明确标记的文本上下文，而不将历史作为 developer 指令或执行工具消息。上下文内容、重复展示过滤、切换失败的提交边界与关联文件结构由 advisor 和源码 owner 核对；不将候选直接提升为已实现事实。

验收要求覆盖至少两种实际运行时的双向接续、多次切换、重启投影、失败保持原会话与下一轮意图、原生管理影响、取消和交互归属，以及元数据文件不包含正文。不新增自动化测试；静态检查、构建和临时 HOME／合成 loopback 脚本，不读取用户真实配置、凭据或会话，不调用真实模型。完成后重新安装 `/Applications/Velune.app`，版本保持0.1 beta.1；真实服务与 GUI 由用户验收。当前仅完成授权与文档归位，源码实施中。

Advisor 决策已采纳：逻辑会话包含有序原生 segment，A→B→A 创建 A₁、B₁、A₂，不向旧 A₁ 补差量。现有 Message.id 在 Pi 有索引 fallback，DSH live 与 history ID 不一致，snapshot.revision 仅生命周期内有效，因此都不能持久化为 cutoff。首版从同一权威历史读取结果保存消息数量与规范化前缀摘要，重开先校验再取截止前缀；截止后的外部追加不进入旧段，截止前改写或删除明确报关联片段不可恢复。投影消息 ID 加 segment 命名空间。

共同公开路径为目标首个用户请求中的引用历史文本与本轮请求。引用保留用户／助手和已完成工具结果出处，不恢复可执行 tool call、审批或签名推理，不将引用提升为系统／developer 指令。当前投影只承诺可携带文本，超长上下文发送前明确拒绝，不能静默截断。关联记录保存可精确识别目标首条交接包的位置与格式标记，重开时从原生正文还原真实用户输入，避免重复展示和递归交接包；不根据任意用户文本前缀猜测。

提交边界：先验证目标、读取完整交接内容并确认无执行或待回复交互，再准备目标；取得原生引用后原子保存关联，再发送本轮请求。准备失败保持来源可读；关联保存失败不能宣称切换成功，也不擅自删除已创建目标；发送结果不明确仍保留关联与目标状态，不自动重发。关联文件独立于普通配置 schema 重置，不复制消息正文。管理采用首段原生标题为逻辑标题权威，重命名调用首段原生接口；删除逐段作用于原生数据，全部确认后删除关联，部分失败保留关系并报告，不能用本地隐藏代替。

验收划分：Mac owner 的实际 AppStore 脚本使用两个独立 Pi 实例证明界面状态、逻辑身份和生命周期；Rust owner 的独立脚本使用不同 family 证明跨协议的 adapter 接续。检查目标仅一次请求且无历史工具执行；重启与再次切换没有递归包；关闭、取消、部分删除与各提交边界失败不造成自动重发。源码 owners 自主完成必要局部决策，具名接口直接协调。

执行责任调整：Rust owner 的初次交付只包含发送与关联骨架，未完成整体投影；root 接任 packages 源码并完成线性逻辑投影、具名上下文与管理能力、独立关联仓库和提交边界。ai_service_audit 冻结 packages，转为仅 `scripts/manual-cross-harness.py` owner并独立跑真实跨 family 验收；session_management 持续负责 Mac，不交叉回滚。新 owner 创建受会话线程数量限制，未创建额外任务；此调整不改变已确认范围。

已实现的具名契约：逻辑会话沿用首段完整公开ID，summary.runtime_id 用于来源分组、打开和管理，snapshot.context_runtime_id 用于尾段查询／取消／回复；summary.can_rename/can_delete 由 application 结合原生能力和启用状态装配。关联 schema 1 保存 link.id 与有序 segments，每段仅有 runtime instance/type、完整native公开引用、可选cutoff（数量＋规范化内容digest）、可选handoff（marker＋完整payload digest＋用户记录序号）及已确认删除标志。每次回切追加新原生段，不改旧原生文件。交接为精确marker包围的JSON历史引用与本轮request，目标只收到原生用户输入；投影验hash后还原request，旧段UI消息加命名空间，不递归展示引用包。

截至debug probe的实际验收：`manual-cross-harness.py` 用实际Pi1.0.2→Codex0.159.3→DSH0.2rc2→Pi，共4次loopback，验证每轮仅一次上游、单逻辑行、origin/runtime/context分离、重开全部文本及稳定ID、metadata无正文、可见user无marker/重复包、首段可改名／含DSH时不能伪删除，以及封闭Pi源追加被排除、prefix合法改写明确拒绝。`manual-session-loading.py` 使用当前AppStore／Transport／UniFFI与两Pi实例，共3次loopback，新增完整逻辑重开、原生首段改名、全段删除、尾段硬链接保护导致部分删除后保留关联与nextdraft、清旧详情禁止发送，去掉保护后同row重试完成。严格Swift warnings-as-errors构建和Python语法通过，Mac源码已冻结。

root的 `manual-continuation-boundaries.py` 用真实Pi和2次loopback，验证256KiB超长及协议不兼容在准备前拒绝、原子关联提交失败不发请求且保来源bytes、显式重试只有一个关联target、元数据无正文/credential且0600、尾段原生历史不可读取时保已校验来源并禁止resume、修复后重开、schema6普通配置hard-cutoff不删除独立关联文件，以及重启不主动发送。Rust fmt/check/workspace strict clippy及bindings no-default strict clippy通过。所有数据均为临时HOME的合成内容，无真实秘密或模型API；release安装与安装库复验尚待收口。
