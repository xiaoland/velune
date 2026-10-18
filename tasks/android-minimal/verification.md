# Android 最小端隔离验收

## 当前结论

2026-10-18，用户已授权按[线性计划](plan.md)恢复实施，S1 已完成当前设备条件下的同网判别，当前进入 S2；E2／E3 继续环境阻塞。下面主矩阵保留前一轮产品观察，不能当作新方案或 S1 已通过；S1 当前事实单列于下方。Android 身份发布修复已提交 `0004d53`。没有将安装、进程存在或临时原生 probe 当作业务通过。

当前 S1 最终接口源码为 `f0df0ca`，真机 APK 为 `8d9f649e…`／so `62bb93ac…`，匹配 host 为 `3b55ead8…`／Python `94570f95…`；完整摘要及版本边界见下方接口补丁小节。三轮正式生命周期已收口，E2／E3 仍缺环境。

| S1 当前边界 | 结果 | 仍缺少的证据 |
| --- | --- | --- |
| 同网显式地址、无公共设施 | 冻结包 159／159 marker；单 Direct／IPv4 连接，missed 与 native dropped 各为 0 | 公网及真实接口迁移 |
| 同网短后台恢复／停止 | 同连接恢复后继续 80.708 秒；UI 停止与 host 正常退出 | 长后台、进程重启、业务恢复 |
| Android mDNS-only 初连与缓存 | 最终接口补丁正常 host 仅 ID 初连通过；三次独立 Wi-Fi 生命周期均合格，FD／锁／host 正常关闭 | 公网、Ethernet、IPv6 路由及业务层尚未覆盖 |
| Host Probe→UniFFI mDNS 缓存 | 同公钥 A→B、Expired、过期后无旧地址 | 不代表手机网络栈或 QUIC 连通 |
| E2／E3 | 用户确认当前只有 Wi-Fi；仅切换既有 Wi-Fi 并恢复原开态 | 受控异网／双网环境缺失 |

下表仅保存前轮产品 UI 的历史范围，不作为 S1 验收结果。

| 边界 | 当前证据 | 结论 |
| --- | --- | --- |
| Android 构建与打包 | app owner 的 assembleDebug／lintDebug 通过；APK 内 so SHA 与修复产物一致 | 通过构建边界 |
| 真机安装／启动 | `adb install -r` 成功，本 app 进程存在，PID 限定日志没有 AndroidRuntime／JNA／UniFFI 崩溃 | 通过安装与进程启动边界 |
| 身份初始化 | 修复 hardlink 后已产生 0600／168 字节 identity；诊断包的新鲜 dump／截图确认错误消失；最终无诊断包待冷启重验 | 修复已落地，最终界面重验待完成 |
| Wi-Fi 配对与路径 | 实际 UI 尝试尚未成功；暂停前未完成输入注入与产品行为的区分，未使用 adb reverse／forward | 尚未配对，原因未定 |
| 会话／消息／停止／审批／问答 | 已准备真实 ApplicationService＋合成 ACP／Claude 协议宿主，尚未通过 Android UI 操作 | 未验证 |
| 后台／断线继续、重连、原请求查询、撤销 | 尚未到达业务阶段 | 未验证 |
| Mac 设置入口 | `d3b039d` 隔离 clone 的真实设置启停／邀请／私有权限通过，详见下文 | Mac 本地入口通过；Mac→手机配对待验证 |

暂停时安装 APK SHA-256 为 `33cb77b3827b4ce870acfc284180fbb605b2203ca08fc72d2c46bcd309eebee8`，包含默认隐藏／可显式显示邀请及禁用自动纠正，不包含临时 native 诊断。其 so SHA-256 为 `2529e9142589abddf980c5c9da44876fcef8d369513337a827b7efb0cd32ba00`，与首次修复版字节完全一致。assemble／lint 通过，安装保留 app data；完整冷启、空数据首次创建与业务验收尚未完成。诊断 APK `ecab486` 仅用于先前定位，不能当成交付包或完整业务证据。

宿主使用匹配 main 集成契约的冻结动态库 SHA-256 `e84fc5b8fc43782e1109924f8f4c232a9b50bd185334b98c16404e8724917a25`，Python bindings SHA-256 `f07a5c7cbba659feabb25184c179b7f089644e7b0cb7f32413bd51fc033b90e4`。

## S1 当前环境与采集边界

2026-10-18 恢复时，公开 adb inventory 确认单台 USB 设备在线，API28／arm64-v8a，系统公共锁定状态为未显示锁屏，原任务 app 进程存在；随后已安装首批 S1 探针并完成下面的 E1 探索；该首批尚未带 PathObserver missed 计数，不作为最终 S1 出口。设备编号只在私有操作记录使用，不写入本报告。未读取账户、通知、SIM／手机号或真实配置／会话，未更改手机网络。

目前已具备 E1 同 Wi-Fi 设备条件；用户此后明确当前只有 Wi-Fi，没有蜂窝，因此受控异网出口、蜂窝默认且 Wi-Fi LAN 保留的 E2／E3 缺少环境。本轮不切换蜂窝，也不从状态栏图标推断可用。S1 会先取得可独立执行的同网、路径观察与最小合成载荷证据；不操作真实 Mac app，也不以旧 Mac bundle 覆盖安装。

使用每轮匹配的 S1 so／Kotlin／host 及观测入口，记录构建摘要、进程内连接 ID、网络代际、path 临时 ID、来源／地址族／selected 变化、分段收发字节与有序合成请求序号。路径迁移探针禁止自动重拨；连接关闭后另记重拨恢复。预算沿用 [acceptance](acceptance.md) 的 10／15／30 秒及 180 秒诊断窗口，不将最终恢复改判预算通过。此阶段探针不替代新配对或完整 UI 验收。

### E1 首批同网探索（非最终包）

使用未提交 S1 首批构建：APK `669d3230a13cbdb2abf23b244a8c770f59017a56e5df65ca74c041ca336f87e9`，APK 内 so `2c7a86490d518fc077915abb449f34545a3ab6024d54ca41389b06a5cf54f012`，Kotlin `29c2bcc170de0b7ee356fe99bfee8714d375fc6f9507ad46cddd9c120c400497`。host 实际冻结 copy 的 dylib SHA 为 `1b9ae2f489ad5380114f9a01918a2498ec32016f69901faee0fb60d4997a74cf`、Python SHA 为 `689cc755eb1f755830eabac0f717acdffd40404221da73c560be9fd0e5ac3610`。构建／lint 通过归 app owner；本项实际覆盖安装及设备执行。

宿主通过 `scripts/manual-android-remote.py --transport-probe` 运行，明确 Disabled／None；不打开业务核心、Harness、真实配置或旧配对。将私有 `target.json` 通过本 app UID 输入 `files/s1-probe/target.json`，启动 debug-only `TransportProbeActivity`。没有 adb 转发、网络切换或隐式重拨；为保持前台观察，仅在自有探针空白处进行轻触，不更改全局屏幕设置。

| 观察 | 实际结果与范围 |
| --- | --- |
| 初连 | 手机同一 elapsedRealtime 域的 connect_started → window_started 为 41ms；这不是 WAN／LAN 切换时间 |
| 连续往返 | 89 次开始、89 次回复，256 字节已知载荷均匹配，未见 marker_failed 或重复回复序号；观测约 89 秒，单连接 ID、Direct／IPv4／path 0 |
| 实际路径 | 私有诊断 remote 均属于本轮显式目标地址；同 path 的 tx／rx 字节从 12,235／10,001 增至 46,506／43,473。结合禁用公网设施、已知同网目标及往返，支持本轮直接路径；未证明接口迁移或公网直连 |
| 往返耗时 | 本次 89 个样本中位 10.703ms、最大 36.554ms；不外推生产 SLA 或公网成功率 |
| 网络事实 | 本轮快照只有默认且 validated 的 Wi-Fi 网络；没有证明蜂窝可用或 E3 双网环境 |
| 事件完整性 | 手机 6 条、host 2 条白名单原生事件，collector dropped 为 0；首批 PathObserver 未暴露 missed，不能用前者证明路径观察通道没有丢失 |
| 停止 | 点击真实“停止探针”，最终记录 user_stop 且 evidenceComplete=true；host 随后 SIGINT 正常退出。此标志只覆盖首批实现所能检查的通道，不弥补 missed 观测缺口 |

私有资料保留在本轮 `/tmp` case（含当前 UI 截图与原始 JSONL，不将原始地址入报告）。Android 事件文件 SHA 为 `cc4c5fdc98e305d82e98f81f91071bb3230210090c515777fc25771f4b80df99`。下一步由 owner 提供带 PathObservation missed／mDNS 判别的新匹配产物，再取得最终同网结果；E2／E3 仍需真实环境与用户网络授权，以上不作为其替代。

### E1 冻结包复验与短后台恢复

本轮使用 S1 冻结 APK `6c56f16ea036dca85e8a2e57f5c493a828d7b14516cf4c764f267df3bd90de97`，实际包内 so `cbd4ea223e9b8a6085c15810fc076c88dd43bd2dc088e38ec24c287213355e8e`，Kotlin `0dcef4fcc3b5992f81e42aebddb765010341ac8ed036d3705d127cfb1f90ae2b`；host 冻结 dylib `035997009484baa0a56c03838b6b203afcb7f0eba580b4f4021f3c44a866240d`、Python `47a46e65b15f2cabcec924c4e773a7073c67810a041532ddf76da2c7e7a85439`。此证据限定于 main 合并前的 S1 冻结产物，不声称后续匹配构建已经验收。

每个独立 case 先 force-stop 自有 app，再开新 host 进程，避免进程全局 native collector 将前轮事件归入本轮。公网 relay／lookup、mDNS 全部关闭，手机网络未改变。

| 操作与证据 | 结果与边界 |
| --- | --- |
| 同网连接与连续载荷 | connect_started→connect_established 48ms；159 次开始、159 次匹配回复，序号连续 1—159，无失败或重复；单 connection ID、Direct／IPv4／path 0 |
| 前台→Home→原 task 前台 | 后台 8,424ms；恢复前台后首回执 36ms，仍为同一 run／connection，没有第二次 connect；恢复后继续观察 80,708ms。只证明同网络短后台恢复，不代表网络迁移或后台常驻 |
| 完整性与耗时 | PathObserver missed 最大 0，native collector dropped 最大 0，两个通道分别记录；159 次 RTT 中位 10.908ms、最大 186.987ms，只是本轮基线 |
| 真实停止与资源闭合 | 点击“停止探针”，user_stop 的 evidenceComplete／shutdownComplete 均为 true；host 随后正常 SIGINT，进程退出 0 |

原始 JSONL 保留在任务私有 case，Android 文件 SHA 为 `1279dde379a4a4e78949f0b24c7e509a1954131395c6a6aa733761f0f6952ffc`。未将原始地址或真实设备编号写入报告。

### mDNS Android 初连判别（原因待定）

同批冻结产物、新 app／host 进程，启动 `--transport-probe --mdns`；核对手机 target 的 publicInfrastructure=false、relayUrls/directAddresses 均为空，只含本轮受控 endpoint ID。没有网络切换或公共设施回退。

本轮有 mdns_enabled 与 connect_started，说明 enable 已返回；6 次周期 resolve 均失败，未出现 discovered／connect_established／marker。约 12,196ms 后外层 catch 记录 mdns_initialization_failed，真实截图同步显示无法启动。该标签同时覆盖 dial 失败，不能据名称断言 enable 失败；现有事件缺少错误种类，已交原 owner 定位。最终 evidenceComplete／shutdownComplete 均为 true，MulticastLock 从 held=true 变为 close／held=false。初连目前失败、原因未定；未据此认定 Wi-Fi 屏蔽多播。 host 仅持久化已连接 peer 的 mDNS 事件；本轮尚未连接，其事件为空不能证明没收到手机广播。原 owner 读完资料后，host 正常 SIGINT 并退出 0；手机事件文件 SHA 为 `1195416b149d9125797612f82985e014a789908ce272fda0d482811fc5c9b65e`。

该轮手机同 peer 广告 A→B 更新、Expired／缓存失效未到达；下述隔离发布器对照补齐这些缓存边界，真实接口迁移仍未验证。独立上游／Mac FFI 缓存实验仅证明各自层的发布与解析，不替代手机同 peer 证据。若需隔离判别，最薄入口是独立合成 publisher 保持同一公钥，更新已持有 UDP socket 的 A／B 广告并最终 drop；手机端须在无业务连接的独立 resolve 观察窗口继续采集到过期，不修改产品身份、不读取秘密。

### mDNS 同 peer 更新／过期的 Host UniFFI 补层

独立私有进程以同批冻结 Python／dylib 开启真实 Probe mDNS；临时 publisher 用唯一合成公钥持有两个实际 UDP socket，仅更新广播 A→B，最后 drop service，没有 QUIC server，也没有业务核心。Probe 的显式 mdns_resolve 首次返回 A（本轮开始后 1,568ms），切换后新查询返回 B 且不含 A（2,395ms）；真实 drain 队列在 6,333ms 收到该公钥的 EXPIRED。之后有界 12 秒 resolve 调用未返回旧地址，而以 CONNECTION／remote_failed 结束，最后 probe 正常 shutdown。

临时 observer 最初用 `EXPIRED` 与生成绑定的完整类名比较，派生布尔错误地为 false；原始事件实际是 `BindingProbeMdnsEvent.EXPIRED`。已修正临时 driver，并保留原始事件与单独的解释结果，未用派生布尔覆盖原始资料。私有事件文件 SHA 为 `2f02348959fe39b921b04a9368b0b46c44cb4603d0f74d273bf2a289be88439e`。本项穿过生产 Probe／UniFFI 的发布解析链，但没有 Android 网络栈／MulticastLock，不替代手机 A→B／过期或连通性验收；ring 没有 Dropped 也不证明上游订阅从未丢事件。

### mDNS 错误观测包复验

源码检查点 `b212e81` 保存本轮冻结源码；构建发生在此前 `09039d2` 的冻结 dirty 工作区。APK SHA 为 `ef8143ffacac7380a2b93c866e853b68037132ce87ba08df65eb22c7fb676556`，包内 so `0a05849872e15f33b09ea0c4ef8a58d706a1c13edce690222f27ea074181b894`，Kotlin `8b5a264242e3f2acc9ea8c86d1418b6cddbeadd5c68dbe229d625621c4a3b98d`；host dylib `012f37bc7c6e5141c33044af663623f8f5f2b23751b0c158b286ebe8dfab2f53`、Python `de9574496bab1cb2cbc44aeb465fd8cb557cb2b7ff1dc54aa32161096c850411`，实际文件摘要均核对一致。安装保留 app data，新 app／host 进程重复仅 ID、无公网设施初连。

手机 Published 为 directCount=1／relayCount=0，host 为 direct_count=2／relay_count=0；这些计数只证明 mDNS service 收到 EndpointData，不证明多播发送或对端收到。5 次显式 resolve 以 OUTCOME_UNKNOWN 结束，最终 probe_failure 为 stage=connect、errorKind=binding_diagnostic、bindingFailureKind=CONNECTION；本轮 11,438ms 内没有发现、连接或 marker，未采到白名单 WARN／ERROR。真实截图显示“mDNS 已启用，但连接未建立”；锁释放及 shutdownComplete／evidenceComplete 均为 true。事件文件 SHA 为 `b334d15e1ecf2d0107477d1428af0664ca21c0344180ac6bd8284ab9d001ffb5`。缺事件本身不证明底层没有错误。

本轮未配置或依赖 VPN；只读路由观察发现既有 utun 与 LAN 接口共存，mDNS 多播目的地的默认 route 指向 utun，手机 unicast route 指向 LAN。route 不能单独证明实际报文路径，因此本轮不能表述为无隧道基线。随后用同一临时公钥、同广告内容和同 marker 协议，仅在隔离 publisher 内增添显式 LAN 多播接口，比较手机 fresh resolve／仅 ID 初连；不改系统路由、VPN 或手机网络。

### 手机默认与显式 LAN 发布器对照

使用 native owner 的临时 Rust fixture：随机 key 只在内存中，真实 iroh Endpoint 接受探针 ALPN，两个持有的 LAN UDP socket A／B 仅转发至该 Endpoint；同一 mDNS 公钥与广告 A 保持不变，命令 EXPLICIT 只在原默认 Swarm 上调用 add_interface_v4(LANIP)。没有产品 Handler／业务、无公网设施，手机始终仅接收 ID。该操作保留原默认 membership／receiver 与 IPv6 状态，准确条件是“默认＋显式 LAN”，不能说仅 LAN 或原默认已退。没有更改系统路由、VPN、进程绑定或手机网络。

| 人工操作 | 手机与宿主实际证据 | 判定范围 |
| --- | --- | --- |
| 默认发布器，手机新进程 | 12,290ms 内没有发现／连接／marker，connect 的 CONNECTION 失败；宿主 echo 0，锁和 endpoint 完整关闭 | 与正常 host 的失败一致 |
| 同 fixture／公钥／广告 A，加显式 LAN，手机再新进程 | 仅 ID 查询获得 A；connect_started→established 543ms；102 次 marker 全部匹配且序号连续，单 connection／Direct IPv4／path 0 | 增添 LAN 多播接口使该拓扑下发现与连通成功；不能单凭此结果定位默认报文的真实出口 |
| 同 publisher 换广告 B | 21 次查询曾含 A；之后 9 次 fresh 查询含 B 且不含 A，仍为同一目标 ID | 手机生产 Probe／UniFFI 缓存更新通过，没有换公钥冒充更新 |
| DROP publisher，保留 Endpoint／两 UDP forwarder | DROP 前最后一次 capture 之后收到目标 Expired；过期后 25 次 resolve 未返回旧地址而以 OUTCOME_UNKNOWN 结束，49 次 marker 仍回复 | 广告／缓存寿命与已有连接分离；没有把解析失败说成连接断开 |
| 真实 UI 停止与 fixture STOP | user_stop evidenceComplete／shutdownComplete 均 true，MulticastLock held=false；fixture_shutdown=true、进程退出 0 | 本轮 endpoint／锁／辅助进程正常闭合 |

所有 selected remote 始终为 A，同一 connection 未重拨，因此 A→B 是发现缓存更新而非 QUIC 路径迁移。102 个往返样本中位 9.2835ms、最大 168.031ms；PathObserver missed 与 native dropped 各为 0，未出现 mDNS ring Dropped。没有将这些观测外推为公网预算通过，也不以 ring 零丢失证明上游订阅无丢失。成功 Direct 页面已留实际截图。

默认 case 事件 SHA 为 `39e64805cb455c16cd646de688d25b38f6be83fa3879ad8d27ae8171ebbb3705`；显式 LAN case 为 `55f2fa83ee6602e1e6eb7f1f5f12a76147bd3bef9121d50e12254780c36465ae`。原始地址、公钥和接口只在任务私有 case；报告与摘要只给布尔／计数。该对照支持 S1 窄补丁的判别方向；产品实现的接口集合生命周期由下述新匹配包另行复验，没有沿用辅助 fixture 的通过。

### 接口补丁与正式 Wi-Fi 生命周期

最终冻结源码保存为 `f0df0ca`，没有因保存提交改变已构建内容。APK `8d9f649e349ca51f9591444519b12738f01e394d3e5038e400692c79deb6037f`、包内 so `62bb93acf4eeeca536281e4b643a6862b5ce59e8133045dc07b79eaa770de31a`、Kotlin `df0699a44af830121c36e2bbaabaae5f6e3e86e3517ce4192882c53d907520a4`；host dylib `3b55ead8dc8772d4291b118f360a0dc82891dae601ea43a06a60fc18e528d345`、Python `94570f9509160c316caeed46dbcd108c5cf54141ca84e8dc0e0bf1711e6da2ce`。实际文件摘要核对一致，assemble／lint 通过归 app owner。主执行者核对 vendor patch 能从固定上游逐字重放，双 LICENSE 一致；Cargo.lock 仅改变 mDNS 来源与 remote 对既有 netdev 的引用，没有版本升级；vendor 定向检查、workspace 与 Android 静态检查通过。

正常 host 自动获取接口，不再使用 EXPLICIT 辅助 publisher。首个探索窗口验证仅 ID 初连、Wi-Fi off/on 的接口集合 1→0→1，App 实际 PID 的 5353 socket 2→0→2→0（最后为真实 UI 停止），同 QUIC 的 path 0→1，恢复 IPv4 回调后首回执 930ms。断网前基线不足 60 秒，因此不计正式三轮。之后一个独立进程纯稳定观察到原 300 秒窗口结束、300／300 回复，没有网络转换，也不计正式轮数。

正式每轮新 host／App 进程，原始 target 仅 ID，公网设施关闭；在 window_started 后先观察至少 60 秒，才关闭当前 saved Wi-Fi，确认空接口集合后恢复开态，再从首个恢复回复起观察至少 60 秒，均在原 300 秒窗口内真实点击停止。USB 保持；没有读取 Wi-Fi 密码、改变 VPN／route／蜂窝。手机时钟是本轮 runId 下的 elapsedRealtime，不从两个设备的单调原点相减。

FD 取证只读取实际 app PID 的 socket inode，与该 PID 的 udp／udp6 表中 5353 端口作内存交集，私有文件仅保存计数与采样时间；没有持久化 namespace 表中其他行。InterfacesApplied 只说明 actor 接受集合，不承担 FD 退出证明；本地接口代际产生的 Expired 也不解释为宿主离线。

| 正式轮 | 断网前基线／恢复后观察 | marker 回复／断网失败 | 资源与范围 |
| --- | --- | --- | --- |
| 1 | 69.455s／99.108s；181.416s 窗内停止 | 171 回复＋5 超时＝176 次，结果唯一且载荷匹配 | 一次 dial／同 connection；5353 2→0→2→0；UI stop 完整／host exit 0 |
| 2 | 65.727s／63.067s；141.434s 窗内停止 | 132 回复＋4 超时＝136 次，结果完整且载荷匹配 | 一次 dial／同 connection；5353 2→0→2→0；UI stop 完整／host exit 0 |
| 3 | 65.372s／69.882s；147.152s 窗内停止 | 133 回复＋5 超时＝138 次，结果完整且载荷匹配 | 一次 dial／同 connection；5353 2→0→2→0；UI stop 完整／host exit 0 |

轮 1 的 IPv4 callback→首回复为 803ms；新 selected path 的观察可能早于该事实回调，因此不硬算一个正向 selected 延迟。所有回复／超时按序号分别保留，没有将无副作用 echo 的不重复提升为业务 exactly-once。轮 1 事件 SHA 为 `f13a6f8257e4630260425d2ba8803e4766fed6980e78d261007558b1a7aafffa`；轮 2 为 `187076c91f66e6b307fa502f0edfdd56a503b1ead2414dc15054f9c32c371f38`。轮 2 IPv4 callback→新 selected 观察 249ms／首回复 439ms。 轮 3 为 548ms／98ms。三轮首个恢复回复之后 marker_failed 均为 0，断网相关超时分别为 5／4／5；payload 错配、重复结果均为 0，native dropped／PathObserver missed 各为 0，未见本地 mDNS ring Dropped。三轮 app PID、runId 不同，每轮独立启动 host，结束均 exit 0；最终各自冻结 dylib 没有仍被加载的宿主进程。

三轮的实际起止按手机同一 elapsedRealtime 域保存，单位 ms：

| 正式轮 | window_started | 真实 user_stop |
| --- | --- | --- |
| 1 | 8421299515 | 8421480931 |
| 2 | 8421941330 | 8422082764 |
| 3 | 8422095982 | 8422243134 |

轮 3 在恢复后完整 60 秒观察之外，另经 Home 进入后台 3,895ms，随后返回同 task／同连接，33ms 后得到回复。锁释放事件位于 background 记录前 2ms，后台最后状态为 held=false，回前台后 held=true；后台接口集合没有被伪造成空，mDNS FD 仍为 2，真实停止后为 0。背景暂停 marker 的行为与断网超时分别记录。正式轮 3 文件 SHA 为 `53098967ba30144decba070f0282685375da2039a3901442fa5497881de8e212`，实际 Direct 页面截图留在私有 case。

这些正式轮只证明现有 Wi-Fi 拓扑下的本地接口消失／恢复、同 QUIC 恢复和资源生命周期，不冒充 WAN→LAN→WAN、UDP 阻断、已知 backup 故障预算、Ethernet 或 IPv6 路由通过。用户确认没有蜂窝，E2／E3 的环境缺口仍在。没有追加自动化测试或真实业务／模型调用。

临时单轮 driver 曾在启动异步阶段读到上一 runId 的已完成窗口：该次采集作废，finally 恢复 Wi-Fi 并关闭本轮 host；它不是产品失败。随后改为先记录旧 runId，等待新 run_open 并固定本轮 runId，才允许网络动作和计时；所有 terminal／marker 同样过滤该 ID。另一次准备阶段因已 force-stop 而读取 PID 失败，未进入网络操作，亦不计正式轮数。临时脚本只执行显式人工协议，不接入自动测试或 CI。

## 环境与验收判据

设备为 API 28／arm64-v8a，屏幕 1080×2340、480 dpi，开发机与设备同 Wi-Fi。只操作本任务的 `app.velune.android`、公开设备能力和合成资料，不读取其他应用、账户、通知或个人文件。package metadata 确认为 minSdk 28、targetSdk 36、primaryCpuAbi arm64-v8a、版本 `0.1 beta.1`。

验收必须经过真实 Compose 界面、UniFFI、iroh 与执行端唯一 ApplicationService，执行合成协议 peer。网络回声、Python 直接业务调用或构建成功不能代替界面验收。邀请只存任务私有临时文件，不进入日志、截图或 Git；不调用真实模型或读取既有 Harness 配置／认证／会话。

`scripts/manual-android-remote.py` 显式创建临时 HOME、合成运行时和已完成的合成会话。`--prepare-only` 关闭唯一 application owner 后将配置根交给隔离 Mac app；前台宿主使用另一目录，避免共开配置根。

```sh
python3 scripts/manual-android-remote.py \
  --root /tmp/velune-android-final-host-fixture \
  --library "$PWD/target/remote-control-bindings/libvelune_bindings.dylib" \
  --bindings "$PWD/target/remote-control-bindings" \
  --resources "$PWD/target/runtime-support"
```

宿主显式采用 DevelopmentPublic relay／discovery 和 `0.0.0.0:0`，不代表产品已部署公共服务。Beta 的普通消息得到 `ACP_ANSWER`，`PERMIT` 请求真实 ACP 审批，`CANCEL` 持续执行至取消，`GATE` 等待私有 `hold` 文件释放。新增 Synthetic questions 运行时复用已有 Claude 协议 fixture，真实审批后要求问题回答；它不调用供应商服务。

## 身份存储失败、实验与修复判据

用户解锁后，真实欢迎页首次显示“无法读取或安全保存设备身份”。当时 `no_backup/remote` 已为 0700 且为空。按 native owner 指定，在本 app 私有目录下创建独立合成 `.storage-probe`：固定内容的 0600 marker 创建成功，同目录 `ln` 发布返回 `Permission denied`，link 不存在。仅清理本次 probe，未读 identity、未改现有权限。原 Rust 身份创建依赖 hard_link，Android 沙箱拒绝该边界。

随后检查临时原生 probe 源码，以本 app UID 对独立 `.rename-probe` 目录实测：`renameat2(RENAME_NOREPLACE)` 在目标不存在时发布成功；目标已存在时返回 `AlreadyExists` 且原合成内容保持不变；目录 fsync 成功。扩展 probe 用两个已同步的 4096 字节 A／B marker 并发争同一目标，结果恰一成功、另一 `AlreadyExists`，重开目标完整等于获胜来源内容。probe 只含固定合成 marker，完成后清理目录与可执行副本。

native owner 据此采用 Android 专属 rustix NOREPLACE 发布，不使用会覆盖目标的普通 rename fallback。修复 APK 覆盖安装后，identity 已发布为 0600／168 字节。曾报告“修复后重开仍失败”，随后发现该证据无效：uiautomator 在 busy 状态可以退出 0 而不更新旧 XML，初始临时 driver 未删除旧 XML、也未验证成功输出。driver 已改为每次先删除旧文件并要求明确 dump 成功；busy 记为未知，使用当前 screencap 交叉确认。新鲜 dump／截图实际确认诊断包身份错误已经消失，不能将旧 XML 当作剩余身份 bug。诊断撤除后的 so 精确等于首次修复版，最终无诊断 APK 仍需冷启、重新创建身份和配对实证。run-as probe 本身只支持其观测边界，不能代替真实 app 域的最终证据。

## Mac 本地入口与集成回归

主执行者在 `d3b039d` clean release clone、全新隔离配置根上完成真实 Mac 设置操作：初始未预建 remote，经设置启用显示“已启用”，独立 AX 停止按钮可达；生成五分钟有效一次性邀请，拷贝入口可见但未复制邀请码；停止后回到未启用且清除邀请。remote 目录／identity 权限分别为 0700／0600。较早的 `235b62b` clone 还在停止入口后经真实聊天界面发送 `MAC_AFTER_ENDPOINT_STOP` 并收到 `ACP_ANSWER`，证明停止入口不关闭业务核心；此项证据只归 `235b62b`。

`d3b039d` 安装前仅检查公开 build manifest，原安装 `3f01f9b`、dirty=false，核实它是构建提交祖先以避免回退 main 能力。正常退出已确认的 Velune 本体后原子安装，codesign deep／strict 通过；安装 manifest 为 `d3b039d`、dirty=false、版本 `0.1 beta.1`，未启动真实 app。安装 executable SHA-256 为 `2457b9b864532716d77d2fdfa1cf5e9978e480d375405a8d307233cc08a5fd6f`，dylib SHA-256 为 `391ba580dec7d4b09c785b78e27a068230cb62ea112b2c0b14d78f4151bb8886`，均与构建产物相同。后续 Android 修复不沿用此清单声称新源码已安装。

主执行者以 main 集成后的 host freeze、临时伪 bundle 和隔离 HOME 显式运行 `manual-provider-configuration.py`，原判据全部通过：合成 API key、私有文件 0600、摘要／日志无秘密、保存／重开／模板和 hard-cutoff。全 workspace locked check、严格 clippy、fmt 通过；没有新增或运行自动化测试。

主执行者另在 `09039d29335bcc98cd38aa4d0c7f7ef1e4b3dbd0` 完成正常 Mac feature 的 build-only 回归：manifest dirty=true（当时根文档与诊断 feature 源码在改）、版本 0.1 beta.1、UniFFI 0.32.2、schema 10。Rust 与 41 个 Swift 源文件构建、codesign deep／strict 通过，nm 未包含 velunetransportprobe。executable SHA 为 `df3d665cc016bea971c60e650ea706f9de3ff54182a9406bab9f1abbe27685e1`、dylib `bab3521bffc671cd5d361c8a6e237cda205561e490ce1ba9ed2c8c5491ae5c25`、生成 Swift `e44122f2b35f577163bd445768dac8e185bdc73ef361d2427ad416293c33570d`。仅构建回归，没有安装或启动真实 Mac app，也不是 clean 交付包。

最新正常 Mac 配套 build-only manifest 为 `fffb1e637e81ca5d366cd2fe83380de47a5a584c`、dirty=true；构建内容随后保存为 `f0df0ca`，不改写原 manifest。版本 0.1 beta.1／UniFFI 0.32.2／schema 10，正常 Rust＋Swift／签名通过，生成 Swift 含 ethernet／vpn，nm 无 probe exports。executable SHA 为 `be1655856891ef4d1f8edfb0bdf58cce2eb62e1cc385e2da0fba4fd70a44d761`、dylib `40fe576cd5147686713965a4a048b156b0047c23b3f1a4e2ef2a92e1348f41a0`、Swift `5f531cb0ea437ab451ca7991afdd3de46fdc09a62ab651b9e7c6f8036e8bf622`。这是当前 DTO 回归证据；未安装或启动真实 Mac app，前述 09039 构建仅留作历史。

## 历史观察与剩余范围

初轮设备处于 secure keyguard，曾仅完成安装与进程观察；用户此后已本人解锁，该前提已经解除。main 集成版旧 APK `3eacf0e86be715cd7dd0744d14978cc3b68c51dab64941fa65c2083c3deb75d4` 的安装／进程日志证据不能用于覆盖当前身份失败。

暂停时手机尚未配对成功、未提交合成业务操作。独立合成宿主已正常关闭 endpoint／ApplicationService，隔离 Mac 实例已退出；未清手机数据。后续恢复验收时按新方案核对实际状态，不沿用旧邀请粘贴流程的未完成观察。跨公网、真实切网、移动后台长期行为及真实模型体验未验证；同 Wi-Fi 不能证明公网路径。
