# Mac 统一问题展示

状态：2026-10-06 实现中。用户授权移除会话列表“历史读取”错误分组及 composer／设置底部操作错误，采用集中问题面板，入口由开发方设计。稳定需求已更新 PRD，不把之前的内联错误布局当成前提。

session_management 继续拥有全部 Mac 源码，贯通 Store、Transport、设置／编辑／导入／发现及窗口场景；root 拥有任务文档、长期设计和最终构建安装。两者不覆盖彼此改动。共享 delegation guide 缺失，仍按 AGENTS 的稳定 owner 分工执行。Ponytail lite 只用于避免新建日志框架等无效复杂度，不替代边界判断。

观察到 AppStore 只有一个 error String，每次 enqueue 自动清空，多个表单共享该字符串；Views 直接把 historyFailures 加进会话 List，composer 和多数表单底部重复显示全文。Transport 已有 typed Diagnostic(kind/detail/code/phase/operationID)，应直接保留结构，不从 localizedDescription 字符串反解析。

采用独立原生“问题”窗口，主窗口、设置及编辑 sheet 提供工具栏入口和数量，增加菜单／快捷键入口。新错误不自动弹窗抢焦点。操作记录在当前应用会话内有界保存、显式清除，成功操作不会顺手抹去旧错误；按运行时维护当前会话列表读取失败，恢复后消解，轮询不重复刷条。原生 List／详情展示来源、时间及安全诊断，可复制；只提供安全读取刷新，不自动重放发送、删除或付费操作。保留字段附近校验、登录进度和成功提示，不建立持久化问题数据库，也不改变 Rust domain／UniFFI contract。

不新增或运行自动化测试。验证用 Swift warnings-as-errors、构建和临时 HOME 的人工脚本，优先实际 Store→Transport→UniFFI 错误和恢复路径；不读取真实配置、认证、会话或调用真实上游。完成后本地提交，重建安装 /Applications/Velune.app，版本保持0.1 beta.1，不发布远端。UI最终体验由用户验收。

已接入集中问题记录：Transport 保留 typed 操作身份，Mac 本地显示中文操作来源；批量失败逐项记录而不拼成一个不可定位的 String。取消旧 global error 字段和各处 SettingsError，原生工具栏入口覆盖设置、编辑、导入与发现。持续 snapshot／认证轮询按活动来源合并，避免每0.6秒刷同一失败；记录上限为100，当前范围不增加可配置保留策略。当前等待 Swift 构建、实际临时 HOME 的 Store→Transport→UniFFI 错误／恢复验收与 sheet 入口可见性复核，尚未安装。

实际 Store→Transport→UniFFI 人工验收已通过：临时 HOME 中非法提供商 endpoint 产生的 typed code／phase／operationID 保真；成功 list 不清除旧操作问题，显式清除生效；临时 helper 失败产生按 runtime 活动问题，重复 list 不刷条，读取恢复后消解。额外隔离 typed 输入检查100项上限和活动去重，不冒充真实轮询。Swift warnings-as-errors 构建通过，加载行为回归待最后复核。

原生观察服务启动失败，无法直接检查 sheet toolbar，未将构建冒充可见性验证。sheet 入口改为其已有标题行中的原生小按钮，主窗口／设置继续工具栏，菜单“问题…”绑定⌘⇧M。UI最终体验仍由用户验收，不扩大到修复外部观察服务。隔离 preview 由启动 owner 负责退出与清理。

源码 owner 已冻结。最终 Swift warnings-as-errors debug 构建通过；manual-problems.py 8项复验通过；manual-session-loading.py 19项通过，包括实际列表／详情选择、原生管理与3次合成上游请求。旧 AppStore.error／SettingsError／“历史读取”分组已删除，不为旧脚本保留兼容接口。投影切换、reset及登录结束消解失效的活动问题，操作失败记录仍保留。隔离预览进程和临时 bundle 已清理；最终 clean release 构建安装待 root 完成。
