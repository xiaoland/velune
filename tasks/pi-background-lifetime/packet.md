# Pi 后台任务与进程寿命

## 目标与边界

2026-11-06，用户询问 Pi 后台任务是否因 Velune 每轮结束就关闭 Pi 而无法运行。诊断后沿既有“后台工作结束前保留执行”的产品契约修正缺陷；不读取真实会话／凭据，不调用公网模型，不控制日常或其它开发实例，不提交。使用 svc-workflow 的关系与生命周期分析；独立 Agent 完成 Pi SDK／插件源码复核后承接 Pi adapter 的完整接入与关键验证，Root负责通用接口与application消费者。

## 已确认

- managed 普通会话的 turn 结束后，application 的 drain_runtime 在 executing 从 true 变 false 时调用 finish_managed_turn，再调用 shutdown_active，PiSession.shutdown 最终关闭 RPC stdin；Client.shutdown 等待约一秒后若未退出则停止进程树。侧聊由独立关闭路径保持，原生提供商会话不因该 managed 收尾条件每轮关闭。
- Session::work 只读取 ActiveState::Native，其余一律空；因此 Pi 的扩展后台任务不进入 executing、pinned、生命周期保护或停止后台工作的投影。不是 Pi 自己在 agent_end 主动退出。
- 本机 pi-background-tasks 2.6.10 的 session_shutdown 会等待正在接纳的任务并停止全部 running tasks；完成通知需原 Pi 会话经 sendMessage 的 followUp 投递。即使跳过 graceful shutdown，进程树结束也会中止其拥有的后台子进程。
- 产品契约 docs/prd/agent-runtimes.md 已要求轮次外任务结束前保留进程／网关；当前技术正文 docs/product-tdd/runtime-execution.md 明示 Pi 尚无轮次外工作接入。这是已知支持缺口造成既有承诺不完整，不把依赖“不报告”当成“没有工作”。没有读取用户真实错误记录，不能把源码路径当作该次具体任务已经取证的证明。

## 修复必须保持的关系

turn 完成只结束父轮次的反馈；实际工作状态决定运行时连接、捕获配置、网关和 SSH tunnel 的释放。Pi 任务状态需经 adapter 提供，application 消费通用工作契约；停止、换模型／运行时、配置重准备、归档／删除、闲置回收与退出沿用同一权威，不能各加 Pi 例外。还需覆盖任务结束后的 followUp 通知／自动新轮次，不能仅在“任务数归零”瞬间关闭接收者。

不采用固定延迟关闭、解析 TUI 状态字符串、仅保留 Node 却释放网关或无限保活全部 Pi 的替代方案。插件2.6.10已有公开文档化 Host EventBus API v1（capabilities/subscribe/list/stop_all 与 changed），通过进程内pi.events通信；Velune自有扩展需将版本化机器数据经既有command／私有status通道桥接到Rust，不能把插件TUI的setStatus文案当作工作事实。

## 实现与验收前沿

- LiveConversation收敛work、work_settled、has_pending_metadata、stop_background；work_settled把“列表为空”与“已确认结束且通知交接完成”分开。Native保留原有行为，Pi由adapter负责公开协议的订阅与同步。未知查询状态保活并阻止重准备，不创建伪任务。
- BackgroundTask允许没有独立conversation_id，Pi shell任务属于父会话，不伪造可打开子会话。application统一消费工作／settled判定：资源寿命、闲置回收、切换／恢复／压缩准入、停止及活动列表；元数据还在途不等于可重启，也不应冒充真实任务。
- Host changed counts与durable terminal barrier／完成通知独立。仅凭running=0不能释放；adapter owner继续核对终态通知／自动followUp的交接栅栏，这是交付必须闭合的前提。
- 验收只覆盖核心受影响层及合成关键路径：父turn结束后任务/网关存活、终态通知可达、完成后释放、显式停止以及无插件路径。复用其它runtime既有证据，不跑四端GUI或日常重装。

## 已取得的验证

- application核心check通过；最终共享动态库已成功构建。当前工作树其它字段较旧验证绑定已变化，实际组合验收从本次动态库重新生成Python绑定，未拿旧绑定的“可导入”当作ABI兼容证明。
- Pi adapter定向Rust不变量2/2、Node语法与fake EventBus协议检查通过；证据 `target/verification/pi-background-lifetime/adapter-validation.txt`，覆盖结构化订阅／查询／精确停止、排除命令和日志内容、未知状态与通知交接、已有历史通知确认。
- 实际application＋Pi1.4＋插件2.6.10＋回环Chat Completions，私有应用／Harness HOME，3例全部通过。父轮次已经Idle时后台sleep仍运行、同一Pi准确PID仍存活、父会话保持活动且可停止；自然完成通知作为client消息进入原生历史并触发后续模型请求；后续轮次完成后原Pi进程确实被释放。同一会话再显式发送一轮可正常结束；另两例验证精确停止不让任务完成、未安装插件可正常收尾。报告 `target/verification/pi-background-lifetime-20261106/application-results.json`，有效日志 `application-check-3.log`；仅回环合成上游，无计费服务或真实账户调用。
- application提交／编辑9项定向检查通过。已有拒绝编辑fixture只记录会改变执行的命令，让新增只读capability查询不被误报为取消／分叉；不更改“拒绝输入不得中断来源执行”的判断目标。证据 `application-admission-tests.log`。
- Mac／Windows打包及SSH资源清单均包含新helper，三者列表一致、所有源文件存在并已实际复制至合成资源目录；`packaging-check.json`。Mac构建脚本语法通过，不据此宣称Windows执行或全平台GUI已验收。改动文档16个相对链接／锚点检查通过。
- 前两份组合失败证据保留但不计为通过：第一次为过期生成绑定造成读取失败；第二次把SDK的session activation关闭事件当成进程退出，未区分新建／首次发送期间的旧activation。最终改用本次fixture记录的准确Pi PID判断真实进程存活与释放，不依据该事件猜进程死亡。

## 当前收尾

通用契约与adapter最大前提的独立审阅均无阻塞。确认Pi1.4的sendCustomMessage在triggerTurn=true时通过_startRun先等待agent_start，再投递初始custom消息的message_start/message_end；正在运行时通知进入followUp队列，保持busy直到最后agent_settled。Client保留事件顺序，故Rust收到通知message_end前已消费agent_start，不会在解除pending时错过自动轮次。无trigger的通知无需等待新轮次；此顺序已回填adapter源码注释。首次Host capabilities在本地EventBus同步回应，无插件的无响应才按不支持；已建立能力后的查询失败保持unknown。

桥接自身的activation销毁责任也已闭合：session_shutdown解除Host EventBus和自身生命周期监听、清请求／重试timer、拒绝在途promise；await后重新检查disposed，禁止旧activation迟到发布或重试。局部fake EventBus追加在途list迟到、失败后的retry清理及无监听／无新请求／无发布三项观察，Node检查通过；正常运行与停止的既有E2E证据复用，不再重跑Rust或平台验收。最终git diff --check通过。受影响Rust文件格式检查通过，native/mod.rs仅有HEAD已存在的两处SideChat格式差异（与本次两处Option包装无关），由native-format-baseline.json确认本轮未新增格式差异，未顺带格式化其它逻辑。

没有提交、安装或退出任何日常／其它开发实例。此次无持久schema修改，无迁移；支持范围为提供公开Host EventBus v1的Pi后台插件，不承诺任意未知扩展的进程内计时器均可被检测。源码与打包清单已就绪，下次日常构建／重装才会进入正在使用的App；未主动执行重装。

## 证据

源码关键点：application local/session.rs:100、local/mod.rs:663/686/710、local/execution.rs:390；agent-runtime pi_session.rs:21/74、client.rs:773。插件源码为已安装包 src/extension.ts:732 起和 src/core/registry.ts:3919 起。此次诊断由直接调用链及独立源码复核支持，无需构建、GUI或重复回归。
