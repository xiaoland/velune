# Mac 窗口消失与实例占用（11月6日）

用户报告窗口突然消失、没有崩溃提示，再次打开提示另一个实例；用户确认没有强退或重装操作。本轮先只读保全进程、锁、系统日志与无标题窗口元数据，不读取会话或凭据；后续用户明确授权正常退出两个开发实例并恢复已安装应用。

## 已确认

- 旧日常进程 PID46369（/Applications/Velune.app）在12:19:56.917被SIGKILL结束。launchd精确写明 `sent by bash[19559]`；这是外部shell发出的信号，用户后续确认同时有另一条Agent在调试Velune，具体shell命令仍未取得。没有对应的新崩溃报告。
- PID17907（12:19:30启动）与19583（12:19:59启动）都直接执行 `./target/macos-dev/Velune.app/Contents/MacOS/Velune`，无诊断flags。系统记为launchedByLS=0。19583 stdout/stderr指向 `/tmp/velune-debug.log`。
- 12:25两个开发进程均未隐藏、已完成启动、regular激活策略，CGWindowList中没有窗口；主线程采样正常在事件循环等待，没有观察到主线程死锁。
- 19583打开并实际持有 `~/.velune/runtime.lock`。锁竞争真实存在，不是磁盘残留锁文件，不能通过删文件恢复。
- 标准build-macos脚本只构建，不启动、不kill。当前dev bundle构建于12:17:27。临时目录同时存在Android／远控调试产物，**不能仅凭这些文件归因某个Agent**；用户已确认另有Agent并行调试。

## 架构判断与当前范围

静态审阅确认窗口生命周期与App级核心独立，关掉最后窗口可继续持锁；锁竞争也不会终止后来进程。零会话窗口的新建窗口菜单依赖focusedTarget，是独立恢复缺口，但不能据此把本次外部SIGKILL诊断成App自身退出／崩溃。本轮尚未修改产品源码；优先确认外部启动／终止来源，再决定实际恢复与必要修复，避免针对不成立的前提加自动重开或定时补偿。

## 证据与下一步

证据目录 `target/verification/mac-window-disappeared-20261106/`：`confirmed-signal-sender.json`、`window-state.json`、`process-launch-flags.json`、两份sample、系统统一日志与 `lifecycle-review.md`。最初取证阶段未发送关闭／杀进程／UI输入；后续恢复操作见下节。

具体bash命令与启动来源仍待并行调试任务自行核对。未获准读取真实Harness会话记录，不为归因扫描用户聊天内容。尚未提交。

## 12:56 恢复完成

用户回复“执行”，明确授权此前提出的正常退出两个开发实例、再正常打开已安装Velune。

- 再次核对17907／19583的PID、启动时间与dev路径未变，19583仍持日常HOME锁。已安装bundle严格签名验证通过。
- 使用既有 `scripts/quit-macos.swift` 对dev bundle路径发送正常Quit，两者都正常结束；没有使用SIGKILL或删除锁文件。锁已实际验证可获取后再释放。
- 通过 `open -g /Applications/Velune.app` 使用LaunchServices后台打开日常版；没有复制／重装产物或执行数据迁移。
- 原生只读窗口元数据确认PID31680来自已安装路径、完成启动、未隐藏，已创建1920×1080主窗口及其它窗口；应用没有被前台激活。系统当前仅一个Velune进程，且日常目录锁仅由它持有。没有读取窗口标题／会话正文，也未注入鼠标或键盘。
- 恢复证据：`recovery-launch.json`、`recovered-window-state.json`。这里只确认正常实例和窗口恢复，未据此宣称历史Tab／group全部恢复，未重跑构建或其它验收。

## 开发隔离治理：已授权实现

用户进一步要求梳理如何根治开发／日常实例串线。已用svc-workflow设计方法及独立只读审查收敛[隔离方案](isolation-design.md)：把运行用途与优化profile拆开，默认开发产物具有worktree应用身份、不同进程名与应用入口强制的私有数据归属；SVC启动／停止消费准确身份，日常打包另行显式处理。也包含defaults、URL handler、原生Agent目录和重装“迁移目录／实际启动目录”一致性。

用户已明确“同意这个方案，可以落地”。应用入口、构建／安装准入与精确进程工具已完成；验收只覆盖隔离关键路径。默认开发目录采用更短的 `~/.velune-dev/<scope>`，避免Unix socket长度问题。

## 13:08 复发及跨任务提醒

日常PID31680在13:08:21再次被SIGKILL，launchd明确sender为bash60048；13:08:23裸启动的旧开发PID60056又持有日常目录。正常退出请求先超过30秒，未强杀；后续确认其已结束且锁释放后，13:15后台重开已安装日常版PID68014，窗口恢复。证据在recurrence子目录。

用户明确授权向另一条调试聊天发送提醒，后补充目标在Pi Agent。已查可访问Codex／ChatGPT列表无对应聊天，未向不明目标发送。已通过目标会话元数据定位“远程控制协议不匹配问题”（01a1a9a0-8fb9-7627-a87c-5cef34922f91）。在没有活跃Pi执行进程时，使用Pi1.4官方SessionManager.appendCustomMessageEntry定向持久化一条可见client提醒（entry0755545a），重新用SDK核对该entry；它进入下一次LLM上下文，没有触发模型轮次、读取或输出其它会话正文，也没有手改JSONL。提醒尚无实时执行回执，证据pi-notice-receipt.json。

## 隔离实现交付（13:40前后）

- `scripts/macos_identity.py`是用途／scope唯一生产者；build的profile只决定优化，默认dev与release均为development，日常产物由package-macos／reinstall显式准备。dev系统ID、进程名、私有目录与协议均分开。
- 应用启动准入早于core／遥测／偏好；元数据矛盾、日常目录及其symlink／祖先／子目录、跨scope及无归属非空目录拒绝。默认目录与Agent配置都私有。全量启动参数白名单经审阅删除，最终仅按用途限制开发检查入口，避免复制命令注册与维护负担。
- macos-dev.py与窄AppKit helper提供受管运行副本、来源指纹及准确身份控制；指纹覆盖Info／exe／嵌入dylib，复用还核对profile与source。构建不更新运行副本，停止只使用fresh身份+出生时间正常Quit。工具没有daemon、按名称kill或强杀回退。
- 日常安装仅接受sealed daily source（指定legacy安装目标显式许可），迁移与启动共同使用OS账号日常目录；已显式把本机package-macos的两个必要环境覆盖从既有release入口复制，其余svc.local配置语义保持，不打印签名身份值。
- 旧未封装的 `target/macos-dev/Velune.app` 和 `target/macos/Velune.app` 在准确路径无运行实例后注销LaunchServices并移除；不会再通过这些旧入口裸启动。没有修改、重装或退出已恢复的 `/Applications/Velune.app`。

验收证据位于 `target/verification/mac-dev-isolation-20261106/`：

- SVC Mac dev构建a34a3e5a（56.6秒）通过；删除额外参数白名单后的窄增量5abb714f（约25秒）通过。没有因本次修改重跑Rust全套、四端UI或完整发布构建。
- 应用目录／身份规则、最终用途检查、native helper准入、安装前置拒绝、launcher来源复用窄验通过。来源检查新增的13项取代原先“重建source仍默默复用”的旧测试预期，其余有效证据复用。遥测Mac入口的两项消费者边界也通过，不启动公网联调。
- 实际两个合成worktree身份A/B启动、各自持私有锁；正常停止A后B与日常68014仍在，错误出生时间被拒绝，日常PID／出生时间／锁全程保持；两fixture最终均正常退出。`native-key-path.json`。
- 只补一个实际窗口观测：开发显示名“Velune 开发版”，1060×760窗口已创建、应用未前台激活；A/B defaults域写入探针互不影响，日常域无探针。`presentation-check.json`。未注入鼠标键盘、未读窗口标题／真实消息、未发送模型请求。
- 开发产物直接指定真实日常HOME的负例以exit2拒绝，在core写入前退出。`reject-daily-home.json`。
- 独立集成审阅未发现推翻四项隔离承诺的阻塞问题；不扩大成OS安全沙箱。合成bundle／HOME与测试LS注册已清理，保留小型脚本／JSON／日志。改动文档中的51个本地链接／标题锚点、Python／shell语法与git diff --check通过。

当前默认worktree另有开发实例由并行任务使用，采用独立开发身份和私有目录；本任务只观测，不退出它。日常PID68014保持不变。开发隔离无需重装日常版才能生效；同UID外部任意PID强杀仍无法由应用拦截。Pi提醒等待目标会话下一次执行时读取。

本次源码范围是Mac上下文／工具链及开发说明；工作树中Rust会话／remote与旧Mac编辑／日期通知修改属于并行或先前任务，保留未纳入本任务验收。未提交。

## 并行任务误恢复 editing／submission 的核对

用户引用另一任务“git checkout HEAD --清掉未提交editing.rs/submission.rs”并询问是否相关。现已确认这两文件与HEAD完全一致，先前工作区中两份dirty delta已不在；gateway-versioned-runner/packet.md也明确记录了误操作。本轮开发隔离没有修改这两文件，先前记录已将其列为保留的旧／并行改动。

本机 `target/verification/build-performance-20261106/windows/source-current.tar.gz`（11月6日00:19）仍含两个完整源码备份。已只提取这两份至 `target/verification/mac-window-disappeared-20261106/lost-editing-recovery/` 并生成针对当前HEAD的补丁，git apply --check通过，尚未回写共享工作树。补丁涉及只读／锁定时仍可编辑并在新会话继续，以及关联会话按边界所在段的有效上下文分叉，避免重复拼接先前段。当前Mac相应提示改动仍存在。它不影响本次隔离已通过的构建和原生进程／目录关键路径，但消息编辑相关的组合状态需恢复后重新确认。不能称为不可恢复丢失。

## 消息编辑恢复与提交

用户已授权“回写并提交”。再次确认两个文件无新改动后应用恢复补丁；本次提交范围仅 editing.rs／submission.rs，不包含隔离、Mac界面或并行网关／remote修改。

窄验发现快照旧补丁把 can_send 当作原地编辑资格，但 edit_message 会重新读取已完成提交准入投影的 current_snapshot，运行中 can_send 为 false，从而错误阻止“先停止再替换”。最终使用 input_access 作为控制权依据，避免将普通发送的瞬时资格混作消息编辑的权限；既有只读原生子 Agent 排除规则保持。恢复的关联分叉只导出边界所在段的有效上下文，与交接源已经携带有效上下文的既有实现一致；同步纠正本函数的过时注释。

验证：复用原生上下文导出／导入已有验收，只补 application 拥有层的9项提交与编辑定向测试（全部通过）。控制权测试覆盖五种受限状态保留编辑但撤销原地替换；编辑测试补查运行中仍可原地编辑、锁定时只能新会话继续、子 Agent 不取得编辑资格，及拒绝请求不取消／更换源执行。证据 `target/verification/mac-window-disappeared-20261106/lost-editing-recovery/restoration-tests-final.log`。不重跑全平台构建、GUI或真实模型调用，不重装／退出日常实例。旧补丁不兼容的失败记录保留在同目录 editing-tests.log，不能作为通过证据。

提交完成：82d8b9b（仅两个Rust文件及其定向断言）；格式检查与git diff --check通过。该提交不代表上面的隔离源码与其它dirty改动已提交。
