# 运行时、会话控制与网络资源复核

本轮以真实调用路径为准复核运行时生命周期。保留另一会话正在修改的 fork／翻译代码，不读取真实配置、认证或历史；不做联网压力、安装、迁移或提交。

## 此前改进清单与当前实现

| 主题 | 实际责任与当前状态 | 历史记录的修订 |
| --- | --- | --- |
| 运行时版本与 provider | `version.rs` 声明具体 variant、权限、继续策略；`native/codex.rs` 在恢复和发送时核对 provider，`NativeSession` 在子进程启动时注入 gateway；application 持有逻辑会话接续。 | 工作树已退役 Codex 0.159，当前专用 variant 是 0.166；旧任务包仍列出双版本支持，不能当成当前事实。真实配置迁移按用户要求延后。 |
| 原生控制权 | `control::Target` 负责目标机上的锁观察，application `ControlWatch` 负责缓存、操作准入；自有 native handle 按执行目标、HOME、variant、agentDir、native ID 核对，避免把自己判成外部控制。 | 运行状态与控制权独立；原生文件残留不等于仍持锁。Mac／Linux 的锁观察证据边界沿用既有 README，不扩大成跨平台已验证。 |
| 停止、继续和插入 | `ResumeStrategy` 区分原生空输入继续、普通消息兜底和不支持；`LiveConversation` 暴露 live steering／停止。Codex 过载恢复由 adapter 持有，等待与重新起轮期间不冒充可 steer。 | Pi 早期“不支持继续”的诊断已被普通 `continue` 消息策略取代；这仍不是 SDK 的原生无消息继续。 |
| Pi 无工作区和模型容量 | application 创建无工作区 Pi 会话时给出目标机 `~/Documents/Velune`；virtual model 在扩展准入前公开已知 contextWindow／maxTokens，未知不伪造。 | 当前源码已承担这两个原因层面的修正；原有 cwd 为 `/` 的记录不被改写。 |
| 命名 | Codex 保留原生命名；其他运行时的可选兜底由 application `naming::Job` 持有，30 秒超时、取消、会话和设置核对，生成结果写既有组织记录。 | 后台命名不通过会话 turn 或新的 gateway 执行；原生名称优先。检查了任务结束和迟到结果条件，本轮未重新请求模型服务。 |
| 原生 fork 与翻译分叉 | 单一原生历史用 Codex／Claude／Pi 自身的分叉接口；关联会话逐段导出有效上下文，经可定位边界截断后导入新的原生会话。 | `reference_context` 塞入 developer instructions 的历史方案已被替代，修正 agent-runtime README 的过时说明。 |
| 子进程与诊断 | 本机 runtime／helper 由 `ProcessTree` 管理独立进程组；stderr 有界、业务 stdout 完整；一次性 helper 有 15 秒期限。网络采集由独立只读脚本和有界轮转日志组成。 | “退出 leader 后还能安全杀 group”的保证须同时覆盖 SSH supervisor 和 tunnel，不能只审本机 runtime。此轮补齐下述缺口。 |

## 已确认并纠正的问题

### 1. Pi 历史查询期间扩展自行启动会漏掉 busy

触发条件是 `sync_history` 开始时空闲，而扩展在 metadata／get_state／get_messages 等待期间发出 `agent_start`。旧代码直接把缓冲事件交给 projection，但没有经过 `poll` 中更新 `busy` 的逻辑，随后还用查询前的 busy 清除取消资格。结果是正文可显示运行中，session 却报告空闲，steering／停止入口消失。

修正将活跃标记与 client／projection 一起归 `Running`，轮询和历史查询通过同一个 `apply_event` 消费生命周期。接受 prompt 尚未收到 start 的 busy 语义保持。新增实际 stdio 合成 helper，在 get_state 前发 agent_start，核对 turn_running、steering、can_cancel 及之后 abort／settled 恢复。

### 2. 远端 supervisor 在停止进程组前回收 leader

原 `remote_bootstrap.py` 的 `Popen.poll()` 和 TERM 后 `wait()` 会先回收 leader，再以旧 PID 发 group KILL；这与本机 `ProcessTree` 已建立的 PID／PGID 不复用约束相矛盾。子进程先退出是正常条件，不需要假定恶意输入；错误投递需要再遇到 PID 复用，不能声称已观察到误杀。

修正为 waitid(WNOWAIT) 观察、停止 group、最后 wait 回收。Linux 使用 Python 的 os.waitid；macOS Python 没有此入口，通过 Darwin 系统 ABI 调用，同样保留 EINTR 重试和 errno。原脚本的 start_new_session／killpg 已要求 Unix 远端，本轮没有增加 Windows SSH 目标支持。

在本机 SDK 头文件通过 C 静态断言核对 `siginfo_t` 大小 104、si_pid 偏移 12、si_status 偏移 20；实际检查观察自然退出不回收 leader、ECHILD errno、自然退出与主动停止清后代、交互 EOF、SIGTERM 和非交互 15 秒超时均通过。脚本和日志为 `/tmp/velune-runtime-lifecycle-check.py`、`/tmp/velune-runtime-lifecycle-check.log`。没有 SSH 连接或网络请求；只创建和停止自身合成进程。

### 3. SSH tunnel 的进程树和结束责任不完整

`LoopbackTunnel` 原先直接保存裸 Child，Drop 只停止 master；SSH 配置使用 ProxyCommand 一类后代时不能继承 runtime 进程树的清理保证。现复用已有 `ProcessTree`，连失败、自然退出和 Drop 都保留同一拥有关系。新增 fake ssh：master 创建同组、忽略 TERM 的 helper，转发／验证命令只返回合成结果，不打开任何连接；Drop 后核对后代不再活动。

另外 `NativeSession` 仅在下一次 prepare_turn 或整个对象 Drop 时释放 tunnel，失败快照或闲置 session 继续持有已结束进程的反向转发。现 fail_connection、release_override、shutdown、Claude turn 自然结束和侧聊取消在相应子进程结束后同时释放本轮 tunnel。未建第二套 owner 或额外后台清理器。

### 4. 运行中 fork 的展示资格与执行资格混在一起

已完成 turn 可以在后续 turn 运行时显示按钮，Mac 可挂起到空闲；但应用直接调用原生 fork 时会释放源 Codex writer。并行 fork 会话在本轮期间加入 `Error::Busy` 和直接 fork 的 core guard，本轮保留采用，没有重复实现。继续核对发现 edit placement=fork 仍能走到 invalidate_execution，已在替换内容和模型校验后加入相同 Busy guard；bindings／MCP 直接调用不再静默中断源。

同一入口此前在检查附件或选择前就 cancel 原地编辑，导致无效附件也会先停止任务。现将 cancel 移到替换内容／设置通过校验之后。Mac 的“当前 turn 结束后自动分叉”保持；远端／MCP 收到可重试 Busy，由调用者在 turn 结束后重试，没有新增核心等待队列。新增实际 Application + Pi stdio 合成 fixture，检查无效原地编辑、运行中 fork 编辑和直接 fork 均保持源 session／运行状态，拒绝路径不发取消或分叉控制。

## 网络记录器的只读与归因边界

静态检查 `diagnose-network-macos.py`：进程读取使用 libproc／ps 的身份与 comm，socket、接口和内核计数来自 netstat／nettop／sysctl；没有发送网络请求或修改系统设置的路径。地址、包内容、命令参数和环境不交付到记录。失败不保留原始命令输出。采样前后再次核对进程身份，kernel last_pid 明确只关联“当前占该 PID 的身份”，不宣称历史因果归属。

`network-flight-recorder-macos.py` 只写自身私有记录、状态和显式安装的 LaunchAgent；默认 16×8 MiB 轮转，上限记录超大样本为错误；观察截止时间与可保留历史长度分别披露。stop／uninstall 是显式管理操作，保留证据；本轮没有调用这些操作，也没有改已安装副本。XNU 私有 ABI 只接受固定版本与长度，变化返回 unsupported／partial，不伪造零活动。

这些保证不能证明历史 mbuf 耗尽由 Velune 或代理导致，也不能排除短命进程、采样间脱组和 kernel last_pid 陈旧。先前资源诊断仍开放。

## 验证与交付状态

- 远端 bootstrap 上述六项合成检查通过；未验证 Linux 真实目标，Linux 路径复用标准库 os.waitid。
- Pi 同步事件定向回归实际运行 1 项通过（33 项 filtered）；Application 编辑准入实际运行 1 项通过（30 项 filtered），覆盖无效附件、Fork 编辑和直接 Fork。两个测试均使用实际 stdio 子进程。
- 无网络 tunnel 后代回归最终实际运行 1 项通过（33 项 filtered）；SSH fixture 为 `/bin/sh`，master、忽略 TERM 的同组 sleep 后代、合成转发和验证回应，全程没有连接网络。最终复用已编译测试二进制输出保存在 `/tmp/velune-runtime-lifecycle-tests/tunnel-final.log`。
- 首轮 Python SSH fixture 在 readiness 前遇到进程启动等待，未验到 Drop；换成上述 shell fixture后通过。Application 测试也曾在 `_dyld_start`、96 KiB 占用下等待，随后原产物正常进入测试并通过；复制后重新签名没有证据能解决此启动等待，未修改系统设置。
- 已运行受影响 Rust 文件格式化和 git diff --check；整个仓库的组合构建由总任务统一进行。
- 未迁移真实配置、重装或退出日常应用，未提交。其他 fork 会话仍可能修改共享 editing.rs，最终交付前须再次核对 guard 未被覆盖。
