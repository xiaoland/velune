# 最小 Android 远控端

2026-10-18，用户在[技术方案](../../docs/design/remote-access.md)、[人工验收方案](acceptance.md)和[线性计划](plan.md)收敛后明确“好的，开始实现”。S1 已完成当前设备条件下的源码与三轮同网生命周期判别，保留 E2／E3 环境阻塞；S2 稳定服务出口已完成，当前进入 S3，S4—S6 尚未开始。用户随后确认设备只有 Wi-Fi，已按技术依赖修订执行门槛：完成现有环境可做的 S1 后进入独立 S2／S3，E2／E3 继续阻塞 S4 出口与最终交付，不标 S1 全部通过。实际观察归 [verification](verification.md)，锁定依赖与机制实验归 [transport-research](transport-research.md)。

## 目标与范围

Android 使用 Kotlin／Jetpack Compose，经 UniFFI 嵌入 remote-only Rust SDK，不携带 application、Harness 或 AI 执行栈。公网与局域网使用同一设备、配对和连接体验；公网进入同 LAN 后自动选择可用的更优路径，离开后恢复，不能用主动反复重连掩盖路径迁移。这里的多路径指候选、主备和自动选路，不要求带宽聚合。

扫码后由执行端本地确认，身份 pin 与方向授权分开；联系人、配对事务、未决请求和恢复循环归 Rust。业务 owner、调用世代、账本和运行任务独立于 Endpoint／网络代际。以上是已收敛的待实施契约，不能视为当前源码已具备的保证。

完整业务能力通过共享 SDK 覆盖；最小 Android 界面先完成设备连接、会话列表／历史、新建及运行时／模型选择、收发／取消、审批和问题回答，不复制全部桌面配置界面。主核心在平台 app 内同进程运行，不另建常驻 Host。

## 工作区与授权

用户要求把工作迁到独立工作区并恢复原目录 main，已完成。本对话附属 `android-minimal` worktree 持有 `feat/android-minimal`；原目录保持干净 main，不用于本任务修改。迁移检查点 `3b8bed7`、早期功能 `235b62b`、旧 main 集成 `d3b039d`、Android 身份发布修复 `0004d53`、设计及计划 `7487a8d`、S1 源码检查点 `7f552cc` 均为本地提交。

允许本任务源码、文档、构建、隔离实验、已连接 adb 真机验收与本地提交；不 push、创建 PR、部署公网服务、读取真实配置／凭据／个人资料／会话或调用真实模型。没有新增或运行自动化测试；以类型检查、静态检查、构建和有界人工合成实验验收。设备认证由本人完成，不绕过系统锁屏。

当前 Mac 安装公开 manifest 为 main `03149ef`、dirty=false、schema10。S1 源码保存后，已在独立分支合入该固定基线，集成提交为 `09039d2`，保留新提供商来源及子 Agent 功能；新公开认证类型继续归中立 application-api，不让 Android remote-only 依赖执行栈。当前只做合成宿主和 build-only Mac 构建，不用旧 bundle 覆盖现有安装。

## S1 当前状态

S1 已实现最薄观察通路：Android 默认网络、可用 Wi-Fi、链路属性和能力变化进入 Rust；路径观察带连接／path 临时 ID、selected、安全统计及原生事件落后计数。可选诊断 feature 提供低流量 marker 探针与私有白名单事件采集；普通产品构建不包含探针。路径探针禁止自动重拨。

mDNS 以官方固定提交 `6f103e7a92ddbd2007383527ab5024c983e3c1c6` 的缓存修复为基础。真实手机正对照已触发接口装配窄补丁，决定只 vendor 该单 crate，保留来源／双许可，不复制成新的发现实现。接口变化仅重建内部 discovery、空集合停用、旧 callback 隔离，Endpoint 与业务连接保持；非空集合保留上游默认接收和 sender 全失败时的默认发送回落，配置事件不证明 LAN 已就绪，不扩到第二份 swarm 补丁。具体源码和生命周期由 native owner 贯通。Android debug 探针按生命周期持有多播锁。mDNS 用于发现和重连，晚 lookup 缓存不等于活动连接已经新增路径。

| 已有证据 | 可以支持的结论 |
| --- | --- |
| 接口补丁的 workspace check／全 feature 严格 clippy、normal remote-only 和 Android diagnostics 目标严格 clippy、fmt、vendor 独立 rustfmt／clippy-driver、API28 native release／生成 Kotlin／Python、Android debug assemble／lint | 中立 API 与 remote-only 边界保持；新包已交真机复验。release 源集隔离不等于已用 normal so 打包 release APK |
| `09039d2` normal feature 的 Rust／Swift／Mac build-only、签名与探针导出隔离通过 | 保留主线后的正常构建边界；manifest dirty=true，仅作回归，不是干净交付包，没有覆盖安装 |
| 最终 missed_events 接口包同网 159／159 marker 匹配、同 connection／path Direct IPv4，后台再返回沿用原连接；missed 与 native dropped 均为零，手机与宿主正常停止 | 该同网产物的直连和短时前台恢复；不能外推公网、换网或完整业务 |
| 真实原生路径队列溢出仍返回 missed_events 与当前快照，原连接继续往返 | 观察落后能够显式报告，不把快照当作完整历史 |
| 实际编译 Kotlin 对 scoped IPv6 的规范化，经 UniFFI／Rust 接收 | 网络事实边界接受规范化 IP，保留 Network／接口归属；不证明 IPv6 路由 |
| 独立 mDNS 实验的同 peer A→B 更新、fresh resolve 及过期判别 | 固定依赖的缓存机制；Android 实际多播与资源管理另验 |

Android mDNS 仅 ID 初连的首轮失败，没有公共设施回落；私有事件已确认 enable 成功后 lookup／dial 失败。阶段／有界错误种类和真实 publish 计数已补齐；发布计数仅证明 service 收到 EndpointData，WARN／ERROR 采集只读固定源码位置，不格式化任意上游原文。后续对照已闭合同 peer 更新与过期。最新接口补丁包为 `8d9f649e`、so `62bb93ac`，正常合成 host 自动采样 LAN 后已仅 ID 连接成功；手机真实 Wi-Fi off/on 探索观察到接口 1→0→1、5353 socket 2→0→2、同一连接 path0→path1 并继续往返。该探索前基线不足 60 秒，不计正式轮次。随后三次独立正式窗口均满足断网前与恢复后各至少 60 秒；分别为 171／132／133 次匹配回复，断网期间 5／4／5 次超时，恢复后未再失败。每轮一次 dial、同一 connection，真实 app 的 5353 socket 均为 2→0→2→0，锁释放、UI 停止与宿主退出正常；轮 3 额外短时后台后沿原连接恢复。具体计时、无效采集与证据边界归 verification。

新的宿主路由观察显示，到手机的单播走 LAN，而 `224.0.0.251` 组播默认路由指向既有 utun。仅 route 查询不足以断定报文路径；同公钥／同广告的临时发布器实测 default 连接失败、加入显式 LAN 接口后仅 ID fresh resolve 与 102／102 marker 成功，触发上述局部修补。手机随后 9 次查询仅返回 B、去掉 A；DROP 后观察 Expired，25 次查询没有旧成功结果，49 次 marker 仍回复。selected 始终 A，缓存更新不等于 QUIC 路径迁移。对照仍保留默认 membership，不能签为仅 LAN 或已退出旧接口；补丁资源寿命尚待验收。全部临时资源已关闭，没有更改隧道或系统路由。此前“无 VPN”只表示本轮未配置／依赖 VPN，不能宣称宿主不存在隧道。

真机为 USB 连接的 API28／arm64，用户已手动解锁；用户现已确认没有蜂窝数据，目前仅 Wi-Fi。本轮生命周期实验仅切换已有 Wi-Fi 并恢复原开态，没有改蜂窝、隧道或系统路由。E2 真实 WAN→LAN→WAN 和 E3“蜂窝默认、Wi-Fi LAN 保留”缺少现场条件，同网恢复不能替代它们。三轮现已完成，继续推进稳定服务的 S2；其状态归属不依赖尚未确定的 Android socket 接入。真实网络判据、预算和责任保留在 S4 出口，不得宣称多路径已通过。

工程预算固定为可达 LAN 升级 ≤10 秒、已有可达备份下故障恢复 ≤15 秒、前台／重拨后新鲜业务响应和原请求查询 ≤30 秒。180 秒只作诊断窗口，超预算后最终恢复仍记失败；原始传输 marker 也不能替代后续业务验收。

## S2 已完成边界

源码核对确认：原 remote State 同持 Endpoint、调用世代与 Ledger，已接纳任务又持整个 State；每次 bind 因而更换世代并保活旧网络。bindings 每次 open 新建 runtime，Mac 停止接入后将远控对象置空。S2 由唯一源码 owner 将这些寿命拆开，并在本地唯一业务 owner 上保留服务，避免只靠窗口缓存维持账本。关闭入站、完整替换网络与最终关闭服务分别表达；Android 释放 ViewModel 的整体关闭语义在本切片保持。

S2 源码／API 已静态冻结：fmt、workspace locked check／全 feature 严格 clippy、remote-only 依赖树／严格 clippy与 Android diagnostics 目标严格 clippy通过，两平台匹配产物已构建通过。core 硬切为 RemoteAccessService，FFI 保留对象名与 open／pair／connect，新增入站启停与网络替换；descriptor／shutdown 返回结果，RemoteRequest 保留独立安全 code。Mac 停用错误向调用方传播，成功后才更改 UI 状态。

local bindings 包装强持缓存 ServiceOwner，Handler 仅持 ApplicationService sender 与 runtime lease，不回指包装对象；已接纳任务持稳定状态直到完成记账。客户端持弱网络代际和可撤回连接槽；原生路径事件流经锁定源码确认不保活 socket。关代取消并等待在途 IO 许可排空，在唯一 lifecycle 锁内完整 close／join／drop 后才新 bind。服务持有生命周期事务 JoinHandle，调用者取消等待不丢屏障；无法证明关闭成功时返回错误。runtime 最终析构使用 shutdown_background 避免 worker 内阻塞，不能把它当作网络关闭屏障。

出口仍需真实 Handler 的延迟写入／丢回执／释放旧网络后原 ID 查询，以及真正 owner 重启拒绝旧 ID。现有脚本只断控制端、收到 receipt 后丢弃，Harness turn 的 GATE 也不延迟 Handler，均不能覆盖 S2；改用 slow ACP 初始化门禁上的真实建会话及副作用计数，隔离阻断网络回执，另以同一服务的双角色场景验证停入站后出站仍可用。关闭返回还需结合旧 FD／端口退出与新 bind 顺序。首轮 core 实验已走真实 ApplicationService＋合成 ACP，bindings 实验已走产品 ApplicationHandler／缓存 ServiceOwner：旧包装释放并 reopen 后同世代查询仍为 Running，放行后 Completed且副作用一次。旧客户端与观察器保留时受测 UDP 端口可重新绑定，只支持该受测端口释放，不能宣称全部 FD 归零。最新版驱动均 exit0：独立 runtime 销毁后旧 ID 精确返回 ServerRestarted，写入数不变；首个 close caller 取消后第二 close 仍等待 gate 中的真实任务完成记账。replace 中途取消缺少自然可控 gate，未作行为覆盖，与 close 共享服务持有事务实现；未为取证增加产品 hook。Mac build-only／签名、Android API28 原生库与匹配 Kotlin／debug APK／lint、host Python ABI 均通过；产物摘要归 verification。没有安装 S2 包或启动真实 Mac，手机仍为 S1 包。

## S3 当前接续

S3 一次硬切 v2 协议、方向授权、配对事务、普通状态与 Rust pending；不动独立 identity schema1。具名 SDK 的副作用采用 RemoteOperation 作用域关联准确 ID，保持 payload 形状，不生成逐响应类型的 receipt，不留 last ID 或自动确认旁路；权威契约已归技术方案。90 个现有 Command 逐项区分观察与副作用，不能复用修订分类或只看名称；认证 Inspect／Poll 等实际持久化动作须纳入，未保存的 import preview 仍可观察。普通全量快照与 pending 文件不携带消息、秘密或完整 Completed 正文。

## 接续与责任

- remote_application 作为 S2 唯一源码 owner，负责 remote 稳定服务、bindings runtime／业务 owner 装配、Mac 接入生命周期及直接调用方；不读取真实提供商来源。基线集成已完成。
- android_native 负责 S1 Rust／依赖／原生构建和匹配绑定；android_app 消费冻结契约并构建 Kotlin／Compose。
- android_verification 独占 adb、手机安装操作、合成宿主及 verification.md；其他 owner 不并行操纵设备。
- 主执行者维护集成、当前 packet、长期知识归属、本地提交、全 workspace 静态检查与 Mac build-only。

基线集成、三轮同网观察及 S2 稳定服务出口已完成，当前进入 S3，继续使用[唯一线性计划](plan.md)。S2 已独立完成延迟 Handler 写入、丢回执、Endpoint 替换后原 ID 查询与真实 owner 重启判别；证据只覆盖记录的同版同条件。若发现稳定服务不得不依赖尚未证明的连接结构，则停止相关改造并重审边界。E2／E3 不删判据、不以 loopback 或“最终连上”放行。
