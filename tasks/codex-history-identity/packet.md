# Codex 历史身份解析修复

2026-10-06 用户报告诊断 `18dbe4187808cd98-7335-4` 的 `read/ambiguous_session`，授权排查与修复。安全日志确认来源为 Codex 0.159.3，helper 原生 ID 匹配得到多个结果；此前运行时选择猜测不能解释这条诊断，也未被证明为原始错误根因。

调查起点：已检查 huihua 0.2.0 公共安装代码：scan 读取最多八条头部记录，Codex metadata 对每个 session_meta 覆盖身份；read 则保留 scan 选定身份。初始待验证假设：原生 fork 会保留祖先 header，造成不同会话误用同一 ID。必须先通过隔离原生 fork 与合成文件判别，不读取用户真实会话、配置或秘密，也不任意挑选重复文件。

root 负责 helper／应用必要修复、文档、静态检查与重新安装；ai_service_audit 只读调查原生 fork 规则与隔离证据，必要人工脚本归其所有。不新增自动化测试；人工验收使用临时 HOME 与合成会话。源码已修复并完成 clean release 构建和安装；实际安装库复验已全部通过，用户真实场景与 GUI 复核仍由用户执行。

判别结果：真实 Codex 0.159.3 的短会话 fork 在前八行没有祖先 header，新旧 helper 都能读取。此前合成 fork 只证明 scan 覆盖身份缺陷，不能证明本次用户来源就是 fork。新的决定性实验是原生 thread/revert：稳定父 thread ID 对应两个真实 rollout，新旧 helper 都重复列出父会话并拒绝读取。官方 rust-v0.159.3 recorder 与 revert 实现确认这是合法原生语义，旧文件和 history_base 前缀不能自行挑选或拼接。

Advisor 建议已采纳：Codex 列表与详情全面以原生 thread API 为权威，DSH 保留 huihua。root 删除试作的 scan 首 header 修补，改用 thread/list、thread/read 与 thread/items/list 的原生分页；共用现有 live item 投影和短生命周期 RPC，不引入常驻 Host、模型要求或任意文件 fallback。sourceKinds 为空会过滤为 interactive，必须显式传全部枚举；modelProviders 为空才表示全部提供商。正在验证实际 bindings 的 fork/revert、原生前缀与分页，尚未安装。

Debug 贯通验收：实际 Pi→Codex→DSH→Pi 的四轮 loopback 接续通过，原生详情、单逻辑列表、重启与稳定投影 ID、封闭前缀校验均保留。DSH helper 的普通／损坏记录、过期引用和跨来源具名诊断脚本通过；原 Codex helper 验收改为 DSH，因为 Codex 产品路径已完全移至原生 API。

最终隔离验收已通过：父会话先发送 retained prefix 与第二轮，原生 revert 排除第二轮后，VeluneApplication 保留首轮且无被撤回正文；重启 application 结果一致。实际 codex exec 的非交互会话被发现，列表只有父、子、exec 三条逻辑行。legacy items 为四条，revert 后父会话两条，子会话超过五十条分页为50＋6，升序与跨页内容正确。旧 helper 对同一原生父来源仍返回 ambiguous_session，证明新旧行为区别。脚本使用显式临时 HOME／CODEX_HOME 和本地 Responses，不读取真实资源。

静态检查通过：cargo fmt/check、workspace 全 features/targets strict clippy、bindings no-default strict clippy、JavaScript 语法与文档相对链接。源码 owner 已冻结；下一步 clean 提交、release 构建／安装和实际安装库复验。

安装进度：clean `b07b07a51e08bb1b66045771b9c440a4e5037038` 已通过 Rust release 与 Swift warnings-as-errors 构建，并安装 `/Applications/Velune.app`。安装器检测到旧应用运行，按用户既有授权正常退出及必要终止后完成替换。manifest 为 dirty=false、0.1 beta.1、schema7，签名验证通过。实际安装库跨 family 四轮接续与 DSH helper 诊断通过。Codex 安装库脚本首次运行发现 optional --helper 对照分支仍无条件断言空列表；这是人工脚本缺陷，不是产品回归，owner 正修复并重新验证。

安装库最终验收通过：修复脚本 optional helper 分支后，使用 `/Applications/Velune.app` 的实际 dylib、重新生成的 Python bindings 与该 bundle 的 Resources，原生 fork/revert/exec、保留前缀、排除第二轮、重启三行及50＋6分页全部通过；省略已删除的 Codex helper 路径也可运行。安装库四 family 接续和 DSH 诊断已通过。此次修复只读原生逻辑历史，不要求模型，不主动启动 turn 或网关；没有读取真实会话或凭据，没有真实模型 API 调用。正式 GUI 和用户故障来源由用户复核。源码安装提交为 b07b07a；后续提交只更新人工脚本与本文证据，不改变已安装产品代码。无 push／PR。
