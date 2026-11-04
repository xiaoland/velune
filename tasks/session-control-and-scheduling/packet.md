# 会话控制权、列表双击与本地定时任务

用户已授权按 [技术方案](design.md) 实施。目标：composer 提前表达原生排他控制，通用到各适配器与控制端；双击会话只重命名、单击打开；Velune 承担本地调度宿主并接管已有任务；bottom drawer 在状态改变时高度稳定。后续明确要求 advisor 评审定时任务 UX，并提醒分层验收。

已确认：锁定保留草稿，不自动发送；退出后无独立后台调度，错过触发不补跑；已有任务先暂停导入，用户在原宿主停用并处理在途运行、确认交接后才可启用。没有提交授权；保留工作区其他任务改动，不重装／退出日常 Velune，不读取真实凭据或会话，不执行真实任务或计费请求。

当前实现：原生 control adapter + application 观察／操作准入与共享 input_access；各端 composer；持久总高度 drawer；SQLite 任务定义／运行账本与统一会话派发；来源只读导入和宿主数据库／在途核对；Mac 管理窗口与 MCP。Windows 管理窗口未提供，PARITY 已记录。

已取得原生双 app-server 和实际 application 合成 E2E；各端构建及 Mac 控制锁／草稿／drawer 验证均有证据。列表还有“重命名取消后单击同一已选行”的最终原生验收。详细边界与日志见 [分层验证记录](verification.md)，不要重复未受改动影响的旧 E2E。

当前结果：advisor 两次评审的关键建议已采纳并实现。默认概览／编辑分离、独立启停、普通日程和核心未来三次预览、真实选项／会话选择、来源实际映射与 revision 覆盖确认、明确接管流程和结果窗口路由均已落地。发现并修复保存期间草稿竞争、无效高级规则草稿归属、取消编辑强制解包崩溃，以及关闭会话窗口后误复用旧模型等实际缺陷。

分层依据：新核心测试 10 项；实际 application 的局部脚本（未派发模型请求）；Mac 原生每周创建／保存／启停、草稿与失败保留、导入对照和资格、无会话窗口时打开结果与关闭重开均通过。列表双击、取消后重复单击同一行、多选、键盘、Option 和拖拽已通过原生验证。只恢复任务窗口也能自行载入配置。原有控制权／调度 E2E 和各端构建复用，见 [验证记录](verification.md)。

交付检查：严格 clippy 已通过，移除临时事件打印后的最终 SVC Mac 构建通过（539dc72c-5e94-47bd-8f96-61599eadb590）；文档链接／格式通过。没有提交和日常重装。验证用两隔离实例已退出；没有待用户决策。Windows 任务管理窗口仍未实现，Windows 原生视觉验收未取得；已在 PARITY 和验证记录明确，不能把跨端构建当视觉验收。

隔离实例：`/tmp/velune-control-clicks/VeluneClickCheck.app` / `local.velune.click-check-20261102` 用 `--preview --check-dock-areas`；`VeluneRuntimeCheck.app` / `local.velune.runtime-check-20261102` 用实际 application，但 HOME=`/tmp/velune-scheduling-e2e-_0f1l90l/home`、VELUNE_HOME=同根 app、OPENAI_API_KEY=synthetic-only。所有任务 paused/needsConfiguration；外部 holder 已正常释放，不 kill 旧 PID。CUA 压缩恢复后先 rewriteDocumentation。

2026-11-02 用户提供实际锁定 composer 截图，反证此前只检查 drawer 外边界的验收不足：一行锁提示占据 64pt 弹性滚动区，产生大块留白，锁定态仍保留编辑焦点框与发送快捷键占位。已撤回“composer 视觉布局已充分验收”的宽泛结论。当前修正：锁定说明移到既有操作行，点按查看完整原因；编辑框几何不因控制权状态变化而改变；锁定无编辑焦点框／发送提示且禁止原生粘贴／拖入，保留选择复制；真正附件／队列区按内容高度限幅。当前 Mac 构建中，后续须用含 locked / idle / running 的合成 fixture 做同一尺寸截图对照，不重复已通过的调度核心和应用 E2E。

截图反馈修正已完成：同尺寸空闲／锁定／运行、空／单行／18 行草稿、760 宽编辑区均已实际截图对照；无附件时编辑框 y=550…715 一致，锁定无框外状态带、编辑焦点框和发送提示。完整原因入口、拒绝粘贴、可选择复制、单附件自然高度已验证；最终构建补验 Cmd+3 聚焦与全选只读草稿、附件移除按钮 disabled。最后宽屏空锁定截图已再次查看。SVC build `94af35c6-da2f-4a37-88da-4fb809c62400` 通过，日志 composer-locked-layout-delivery-build.log。已关闭唯一 composer-layout 预览并清理合成附件；无提交、无日常重装。
