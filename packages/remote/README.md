# velune-remote

通过 iroh 1.3.0 的端到端加密 QUIC 连接，将 `application-api` 的具名命令交给执行宿主唯一的 `Handler`。本包不依赖 application、Harness 或 LLM 执行实现，也不读取 `HOME`、`VELUNE_HOME` 或现有认证配置。

宿主显式装配 `DeviceIdentity`、`DeviceStore`、`EndpointConfig` 和可选业务处理器。执行宿主提供处理器；只控制其它设备的 app 可以不提供。GUI 与 headless 使用相同入口。本地 UI 与网络处理器必须共享同一个 application owner，不能为每条连接重新打开配置根。

## 连接与公网设施

`RemoteAccessService::bind` 装配稳定服务并创建首个网络代际；也可先 `new`，再调用 `replace_network`。`descriptor()` 返回当前网络的公开拨号信息，没有已应用的网络时返回错误，不能据此取得业务权限。默认不启用公共 relay 或地址发现。`RelayConfig::Custom` 接受自部署 relay 地址；`DiscoveryConfig::CustomPkarr` 使用自部署 pkarr 服务发布、查询设备地址。持有最新 descriptor 时也可以不启用发现；设备换网或 relay 地址改变后，长期保存的拨号信息需要地址发现服务更新。

服务持有唯一业务 Handler、incarnation、接纳账本与完成任务，宿主保持同一 Tokio runtime 直到 `close().await` 完成。`set_control_ingress(false)` 串行封闭新入站接纳、清除邀请并关闭入站连接，保留本服务已有与新建的出站控制连接。再次开启入口不改变 incarnation 或账本。`replace_network(config).await` 先校验配置，然后封闭旧代、关闭并等待全部旧网络任务与临时 IO 引用，释放旧 Endpoint 后才绑定下一代；此屏障与最终关闭由同一锁串行执行，同身份不会并行发布两份网络。替换失败留下无网络状态，调用方可明确重试新配置。

`RemoteClient` 仅弱引用网络代际；它和路径 observer 共享可撤回的原生连接槽。替换或关闭会清空槽中的原生引用，旧 client／observer 包装对象继续存活也不保留旧 socket；observer 返回关闭快照后结束。已接纳执行任务只引用稳定服务，不引用 Endpoint。`close().await` 是最终服务关闭屏障：停止网络并等待已接纳任务写回账本，不调用 application shutdown。释放对象时的非等待清理只负责请求退出，不能代替这个屏障。进程崩溃与业务 owner 真正重启仍使用下述新 incarnation 语义。

完整替换或关闭事务的任务句柄保存在服务中；调用 Future 取消后，后续生命周期调用仍先等待同一事务，不能跳过退休过程或业务完成记录。宿主在此期间继续保活 runtime。事务正常失败允许下一次显式重试；内部任务异常退出则返回 `Unavailable` 并禁止再绑定，不能把未完成的屏障报告为成功。

`DevelopmentPublic` 是对 n0 公共开发设施的显式选择，不是 Velune 正式服务承诺。正式产品可由宿主装配开箱即用的服务地址，同时允许用户替换为自部署设施；本包不部署服务。地址不接受用户名、密码、query 或 fragment，避免把认证材料混入公开拨号信息。生产服务应使用 HTTPS。

默认 `PathPolicy::Automatic` 保留 iroh 的 IPv4／IPv6、relay 协助建连、直连探测与路径选择。relay 不是固定的数据路径；建立直连后由 iroh 选择实际发送路径。`RelayOnly` 清除直接 IP 传输，只用于受控诊断。`report_network_change(snapshot).await` 校验平台网络事实并转发原生网络变更通知，返回输入序号只表示通知已接收，不证明发现新地址、重绑 socket 或恢复路径。平台负责订阅默认网络与仍可用的 Wi-Fi，保留原始事件时间。`network_change().await` 仍可用于不携带事实的原生通知。

`Controller::connection_status()` 返回当前路径及选中状态；`observe_paths().next().await` 先返回快照，随后按原生路径事件更新。事件积压时重新读取完整快照。快照包含进程内连接编号、连接内 path 编号、单调采样时间、地址族、路径类别、RTT、字节数、丢包及公开设备身份，不包含网络地址或业务正文。原生接口不提供候选来源，因此 source 为 Unknown；Direct 不等于 LAN。每次新拨号分配新连接编号，observer 保留原编号。每条业务请求使用独立 QUIC 双向流；大附件读取不会占住同一连接唯一的业务流。

## 隔离传输诊断

显式 `diagnostics` feature 提供 `TransportProbe`，使用独立 `velune/transport-probe/1` ALPN、进程内临时密钥和最多 1024 字节的合成 marker。它不接业务 Handler、不打开身份／信任文件，也不自动重拨。探针最多保留 16 条连接，每连接最多 8 个 marker 流；每次回执核对序号与完整 payload，路径统计另计实际传输字节。

`ConnectionDiagnostics` 显式包含受控本地／远端地址，只能存入私有验收资料。该 feature 用于复验平台发现与传输边界；正式产品装配不启用它，不能把 marker 成功作为业务授权与恢复闭环的验收证据。

`enable_mdns()` 只在没有其它 lookup 的独立探针上显式启用；默认探针仍不含 mDNS，以免混淆平台地址变化／QNT 的判别。诊断依赖采用 [本地单 crate](../../third-party/iroh-mdns-address-lookup/README.md)，基于 `iroh-mdns-address-lookup` 0.6.0 官方提交 `6f103e7a92ddbd2007383527ab5024c983e3c1c6`，保留同 peer 缓存覆盖修复与默认发现周期。接口集合变化会替换内部 discoverer，并用代际拒绝旧 callback；Endpoint、服务与订阅保持稳定。IPv4 接口为空时不创建 discoverer，显式 IPv4 模式不启用 IPv6。当前只装配诊断探针，尚未装配生产远控服务。`mdns_resolve` 最多八个并发调用，等待上限十五秒，只查询此服务，不建立连接。`mdns_events` 排空本地有界事件队列。Published 记录服务实际收到 EndpointData 的时间及 IP／relay 数量，透明转发原数据；它不证明报文已发出或对端已收到。Dropped 仅计自上次排空后本地溢出或超限项，不代表上游二十项 subscription 队列没有丢失。存在 active resolve 时，上游可能只发送 resolver 更新而不发被动 Discovered，缓存更新必须以再次 resolve 核对。关闭探针清空 lookup registry、释放自持 service 并停止订阅任务。

## v2 配对与方向授权

普通远控协议、ALPN 与邀请格式统一为 v2，只有 `velune/remote/2`；本包没有 v1 兼容分支。`PairingOffer::decode` 是最多 1024 字节的离线结构化解析，只接受 Velune 配对用途、固定目的公钥与少量启动线索，不执行 URL 或命令。邀请不含执行 incarnation。`create_invitation` 同时开放一个窗口，最长十分钟；重新生成关闭旧窗口与未提交候选。邀请包含秘密，不实现 Debug，也不写日志。

`begin_pairing` 经认证 TLS 连接证明邀请，双方独立计算八位核对码。label 固定为 `EXPORTER-Velune-Pairing-v2`，131 字节 context 按协议版本、issuer/controller 公钥、invite ID、attempt ID、新挑战及控制方向编码；不使用 0-RTT 或对端提供的显示码。同窗口最多五次挑战，取消、断线或换公钥不返还预算；同邀请的重复导入复用原连接／attempt。核对码只绑定这次 TLS，原连接确认关闭后不能继续批准。

控制端原子保存目的设备 pin 和待确认 attempt 后才发送 Ready。执行端本地 API `approve_pairing(attempt, expected_peer)` 只允许 AwaitingApproval，原子提交入站 grant 与 completed attempt 后才开放业务；`reject_pairing` 返回实际已提交或未授权状态。批准不是远程业务 Command，GUI/headless 使用相同的可信本地 API。已提交结果可在新连接及执行服务重启后按原 attempt 查询；取消不撤销已提交 grant。`revoke_control` 先持久撤销，再拒绝接纳／查询并关闭该方向的既有访问；旧事务不能重新授信，先前已接纳工作继续完成。

目的设备联系人 pin 与本机授予对端的 incoming grant 分开保存。B 扫 A 并获 A 批准，只允许 B 控制 A；A 不会同时取得控制 B 的权限。名称只是标签，公钥来自已完成握手的真实 peer。`connect` 使用保存的 pin/locator，认证 Hello 给出本次 incarnation；联系人不保存 incarnation，也不因地址或 relay 改变而换身份。

`DeviceIdentity::load_or_create` 只访问显式身份文件，继续使用 identity schema 1。既有损坏身份阻止启动，不重生成私钥。`DeviceStore::open` 使用单一普通状态文件 schema 2，保存联系人、入站 grant、未决元数据与重置通知。旧普通 schema 原子初始化为空状态，不推导授权、不迁移 pending，也不保留备份；通知跨冷启保留，按准确 ID 确认后才删除。平台在清理自己旧的普通文件前调用 `record_legacy_state_reset`，先保留通知；它不触碰身份、Harness 文件或真实会话。Unix 私有文件为 0600，非普通文件或开放权限拒绝，发布先同步内容、原子替换再同步目录。

## 请求交付与操作作用域

`Command::delivery_class` 穷尽分类全部命令，独立于业务修订检查。观察可以重新发起，Effect 必须通过 `RemoteOperation` 或 Controller 的显式 submit，进入唯一写前门禁。`prepare_operation` 固定目的公钥、认证 incarnation 与完整 RequestId；实际调用固定 view/execution 后先原子保存安全元数据，成功才发送。未调用的准备不落 pending；每个作用域只允许一次 Effect。普通未绑定作用域的远程具名 Effect 在发送前拒绝，本地业务 SDK 不受此限制。

pending 最多 256 条，文件与普通 snapshot 只含 ID、CommandKind、view/execution、必要 runtime/conversation 定位和具名查询状态，不保存消息、附件、认证材料或可重放 Command。完整 Completed payload 只由显式查询从执行端有界内存返回，不落盘。状态写入或容量检查失败时不发送；确认落盘成功后才移除准确 ID。作用域 drop、普通方法返回、窗口关闭都不自动确认。控制进程冷启重新连接后只查询原 ID，不自动重发。

稳定服务为每个目的公钥持有一份 Controller；它共享连接与请求 session，窗口只持 view 和编辑基线。S3 提供显式连接／断开、pending 查询与确认，尚未启动 S4 的自动恢复调度。断开意图与忘记联系人不同；有未决请求时不能忘记该联系人。`read_state`／`observe_state` 返回带递增版本的完整安全投影，观察落后重读快照；释放观察不关闭服务。

执行端以设备公钥、client namespace 与 sequence 接纳、去重及查询。重复同标识同命令只返回原状态，异命令返回冲突；有界缓存回收后仍保留接纳水位并返回 ResultExpired。Unknown 表示本 incarnation 没有接纳证据，不能当作允许盲目重发；真正业务 owner 重启后旧 ID 返回 ServerRestarted。这里不承诺跨执行宿主崩溃 exactly-once。

## 当前边界与验证

| 边界 | 限额 |
| --- | --- |
| 已授权单帧 | 32 MiB；20 MiB 内容经契约 base64 编码后可以传输 |
| 未授权配对帧 | 16 KiB |
| 在建／在用入站连接 | 64；端点注册连接共 128 |
| 同连接请求流／全局请求处理 | 8／8 |
| 已接纳未完成请求 | 64 |
| 结果缓存 | 256 条，合计 32 MiB，先达到者回收 |
| 请求 namespace | 每次执行端启动最多 4096 个，同 peer 的 Controller 复用 session |
| 联系人／入站授权／未决记录 | 各 256 |
| 配对窗口／每窗口挑战 | 1／5 |

调用、帧、版本、邀请、存储与授权错误使用无正文的 `RemoteError`；显式业务错误仍由 application 契约返回。完整业务写入冲突、陈旧审批、附件内容边界与每设备独立视图由执行 owner 负责，不由传输 mutex 代替。

2026-10-17 的旧协议合成验收经真实 iroh loopback 选中 Direct，并经显式公共开发 relay 的受控 RelayOnly 连接选中 Relay；这些传输结果不能替代 v2 授权和 pending 验证，也不证明跨 NAT 直连、真实切网或移动后台寿命。早期完整应用闭环见[远程控制验证记录](../../tasks/remote-control/verification.md)。v2 的方向授权、写前存储失败、准确 ID 确认、冷启动查询和真实 ApplicationService／合成 ACP 验收归属[Android 最小端验证记录](../../tasks/android-minimal/verification.md)。本包没有自动化测试或 CI 测试入口。

诊断探针的 Android 接口集合来自最新 `PlatformNetworkSnapshot`：选择真实 Wi-Fi／Ethernet 且非 VPN 网络的 IPv4，不要求它是默认网络或具有 Internet／VALIDATED。首次事实到达前保持空集合。`ethernet` 与 `vpn` 是平台事实，与现有 Wi-Fi／cellular 字段并列。桌面合成宿主通过 `refresh_mdns_interfaces()` 重新枚举 UP、支持 multicast、非 loopback、非 point-to-point 接口的 IPv4；调用方负责有界周期采样，不按路由或接口名字推测 LAN。

InterfacesApplied 的私有地址、时间及 generation 只证明 actor 已接受集合并请求旧 discoverer 退出，不证明其 FD 已关闭。接口代际变化会清除旧 scope 的 peer 缓存并发 Expired，不能据此断定远端离线。已有 resolve 保持其期限，可收到新代际结果；已有 QUIC 连接不随缓存清除而关闭。实际旧资源退出和 IPv4 恢复须单独观察。底层 swarm 仍有 wildcard 接收与接口发送失败行为，接口集合不是流量白名单；边界见 vendored README。IPv6-only mDNS 不在当前修补覆盖内，QUIC 的 IPv6 路径能力不因此改变。
