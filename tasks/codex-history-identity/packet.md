# Codex 历史身份解析修复

2026-10-06 用户报告诊断 `18dbe4187808cd98-7335-4` 的 `read/ambiguous_session`，授权排查与修复。安全日志确认来源为 Codex 0.159.3，helper 原生 ID 匹配得到多个结果；此前运行时选择猜测不能解释这条诊断，也未被证明为原始错误根因。

已检查 huihua 0.2.0 公共安装代码：scan 读取最多八条头部记录，Codex metadata 对每个 session_meta 覆盖身份；read 则保留 scan 选定身份。待验证：原生 fork 会保留祖先 header，造成不同会话误用同一 ID。必须先通过隔离原生 fork 与合成文件判别，不读取用户真实会话、配置或秘密，也不任意挑选重复文件。

root 负责 helper／应用必要修复、文档、静态检查与重新安装；ai_service_audit 只读调查原生 fork 规则与隔离证据，必要人工脚本归其所有。不新增自动化测试；人工验收使用临时 HOME 与合成会话。现阶段没有更改产品源码。

判别结果：真实 Codex 0.159.3 的短会话 fork 在前八行没有祖先 header，新旧 helper 都能读取。此前合成 fork 只证明 scan 覆盖身份缺陷，不能证明本次用户来源就是 fork。新的决定性实验是原生 thread/revert：稳定父 thread ID 对应两个真实 rollout，新旧 helper 都重复列出父会话并拒绝读取。官方 rust-v0.159.3 recorder 与 revert 实现确认这是合法原生语义，旧文件和 history_base 前缀不能自行挑选或拼接。

Advisor 建议已采纳：Codex 列表与详情全面以原生 thread API 为权威，DSH 保留 huihua。root 删除试作的 scan 首 header 修补，改用 thread/list、thread/read 与 thread/items/list 的原生分页；共用现有 live item 投影和短生命周期 RPC，不引入常驻 Host、模型要求或任意文件 fallback。sourceKinds 为空会过滤为 interactive，必须显式传全部枚举；modelProviders 为空才表示全部提供商。正在验证实际 bindings 的 fork/revert、原生前缀与分页，尚未安装。

Debug 贯通验收：实际 Pi→Codex→DSH→Pi 的四轮 loopback 接续通过，原生详情、单逻辑列表、重启与稳定投影 ID、封闭前缀校验均保留。DSH helper 的普通／损坏记录、过期引用和跨来源具名诊断脚本通过；原 Codex helper 验收改为 DSH，因为 Codex 产品路径已完全移至原生 API。

最终隔离验收已通过：父会话先发送 retained prefix 与第二轮，原生 revert 排除第二轮后，VeluneApplication 保留首轮且无被撤回正文；重启 application 结果一致。实际 codex exec 的非交互会话被发现，列表只有父、子、exec 三条逻辑行。legacy items 为四条，revert 后父会话两条，子会话超过五十条分页为50＋6，升序与跨页内容正确。旧 helper 对同一原生父来源仍返回 ambiguous_session，证明新旧行为区别。脚本使用显式临时 HOME／CODEX_HOME 和本地 Responses，不读取真实资源。

静态检查通过：cargo fmt/check、workspace 全 features/targets strict clippy、bindings no-default strict clippy、JavaScript 语法与文档相对链接。源码 owner 已冻结；下一步 clean 提交、release 构建／安装和实际安装库复验。
