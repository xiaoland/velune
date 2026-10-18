# iroh 1.3.0 接入设计证据

检查日期：2026-10-17。Cargo.lock 锁定 iroh／iroh-base／iroh-relay／iroh-dns 1.3.0；iroh 源码提交 `0072d7d84b233f9e7185eb676f049beaf557ac03`，iroh-dns 发布源码提交 `17c0612f80f78f5288e97b818b1360ae6ea0a51a`。本记录区分源码事实、本机实验和拟议行为；方案归 [统一设备远控](../../docs/design/remote-access.md)。未操作 adb、启动真实 app、部署服务或修改产品源码。

## 设施更新与寻址

| 核查项 | 锁定版本的事实 | 设计含义与来源 |
| --- | --- | --- |
| relay 热更新 | insert／remove 修改共享 map，异步触发 net-report，不关闭 Endpoint | 可热改 home 候选，不构成切换事务；[公共 API](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint.rs#L995) |
| Disabled → 启用 | Disabled 在 bind 时不创建 relay transport | 只 insert 不能补建 transport；[装配](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/transports.rs#L225) |
| 删除 relay／更换 token | 旧 ActiveRelayActor 不立即终止，token 在创建 actor 时取快照 | 不是严格断流／凭据撤换；[actor](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/transports/relay/actor.rs#L1232) |
| 清空 relay map | net-report 可能直接成为 None，不通知 relay transport 清 home；Some 无 preferred 时也可能保留旧 home | 不等于 Disabled；[空 map](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket.rs#L793)、[保留 home](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket.rs#L1960) |
| lookup 热更新 | registry 支持 add／clear，add 收到最近发布资料；无同步发布完成确认 | clear 不撤销旧 path 缓存或已物化查询流；[实现](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/address_lookup.rs#L492) |
| 旧 EndpointAddr | 加入既有候选后仍可查询 lookup；有 selected path 或已有查询时跳过 | 陈旧地址不是必然压制发现，新候选是增量加入；[控制流](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state.rs#L869) |
| 地址新鲜度 | last_updated 目前未用于更新决策，DNS／Pkarr resolver 填 None | DNS TTL 不是 Endpoint peer cache 的刷新保证；[字段](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/address_lookup.rs#L380) |
| relay 配置范围 | 本端 map 是 home 候选及 token 来源；可以拨不在 map 中的对端 advertised relay | 不等于出站白名单，也不需 relay 间互联；[路由选择与 token](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/transports/relay/actor.rs#L1274) |
| relay 认证 | 原生 WebSocket upgrade 可带 Bearer header；公开 EndpointAddr 只有 URL | 当前 Velune URL-only 配置尚未提供 token 装配，不能声称任意受限 relay 都可用；[header](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh-relay/src/relay_map.rs#L258) |
| UDP 不通 | relay 走 TCP／TLS／WebSocket；默认 HTTPS 延迟探测支持在 QUIC 不通时选 home | 仍要求相关服务与 upgrade 可达；[探测目的](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/net_report.rs#L89) |
| 本地发现 | Minimal 只选加密 provider；N0 加公共 relay／lookup，均不含 mDNS | mDNS 为独立 crate，不能把默认预设当作离线 LAN 发现；[预设](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint/presets.rs#L45)、[mDNS 官方项目](https://github.com/n0-computer/iroh-address-lookups/tree/main/iroh-mdns-address-lookup) |

HTTP Pkarr 按 EndpointId 验签，解析函数没有最大记录年龄或相对上次版本的单调新鲜度检查；n0 DNS TXT 的该客户端查询路径不做本地 Pkarr 验签。最终 QUIC 身份校验仍是信任边界。默认 RR TTL 30 秒只约束 DNS 缓存；发布器地址变化后异步更新，默认约五分钟重发，停用 publisher 不等于删除服务端记录。[发布与解析](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/address_lookup/pkarr.rs#L119)、[验签](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh-dns/src/pkarr.rs#L90)。

PkarrPublisher 默认 `relay_only`。当前 Velune 的 DevelopmentPublic 和 CustomPkarr 都没有覆写 filter，因此禁用 relay 后不能假定其已发布直接 IP。[filter](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/address_lookup/pkarr.rs#L161)。

只持 EndpointId 可向已配置 lookup 查询；ID 不包含自定义 lookup 的地址。换 relay 可保留旧 lookup 发布新 home；撤掉整套旧 lookup、而离线端从未收到新入口时，无法只靠 ID 找回新设施。这是信息缺失，不能通过增加重试消除。

## 本机判别实验

临时 Rust 程序复用锁定依赖，使用合成身份、MemoryLookup 和本机直接地址；没有新增自动化测试或提交实验代码。先给 A 一个 B 的错误地址 `127.0.0.1:9`，由计数 lookup 提供 B 的实际地址并完成连接；然后在原 A 上 insert／remove 一个不可达 relay、clear／add lookup，使用原 QUIC 连接收发已知载荷。

| 观察 | 结果 | 适用边界 |
| --- | --- | --- |
| Custom(empty) bind | true | 与公开 builder 注释不一致；方案不依赖该文档外行为 |
| 陈旧地址下调用 lookup 并建连 | true | 仅本机候选查询，不证明公网更新成功率 |
| 更新 map／registry 后 EndpointId 不变 | true | 不能证明旧 relay actor 已停止 |
| 更新后原连接的已知载荷往返 | true | 原路径为本机直连，未实测新 relay 的转发切换 |

该实验佐证 API 更新可保留对象与连接，不验证跨 NAT、真实切网、实际 relay 撤换或移动后台。

## 活动连接的多路径

本节补充固定 noq／noq-proto 1.3.0 提交 `c1f411562e6078852749b8bcf1190523096a107f`，mDNS 0.6.0 提交 `c64aec6e2afb0d0b3563858a58e70a00b8bfa94e`。mDNS 尚未纳入产品 Cargo.lock，不能把其元数据兼容当作已构建可用。

| 核查项 | 源码事实与边界 |
| --- | --- |
| 默认选择顺序 | IP 为 Primary、relay 为 Backup，跨档优先级高于 RTT；同档比较平滑 RTT，IPv6 偏置 3ms，改善至少 5ms 才切换。不直接评分丢包、带宽、费用或 LAN／蜂窝身份；[selector](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/biased_rtt_path_selector.rs#L79) |
| 重选与保留路径 | 新连接、路径 Established／Abandoned 触发选择，不是每次 RTT 更新重选。选中路径为 Available，其余为 Backup，客户端可能关闭冗余 IP 路径；[调用点](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state.rs#L648)。iroh 至多选一条主路径，底层 noq 的多 Available 调度不等于产品已支持聚合；[单选](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state.rs#L1472) |
| 晚到 lookup | selected path 抑制新 resolve；仍存活的流返回新地址只更新候选缓存，没有打开路径／触发 QNT；[控制流](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state.rs#L882) |
| mDNS 窗口与缓存 | resolve 只保持 10 秒，之后的发现进入独立 subscribe，并不自动注入 Endpoint；[实现](https://github.com/n0-computer/iroh-address-lookups/blob/c64aec6e2afb0d0b3563858a58e70a00b8bfa94e/iroh-mdns-address-lookup/src/lib.rs#L388)。已有 peer 更新使用 entry.or_insert，可能事件为新值而 book 仍旧值；属于源码风险，未做实际 mDNS 实验；[更新](https://github.com/n0-computer/iroh-address-lookups/blob/c64aec6e2afb0d0b3563858a58e70a00b8bfa94e/iroh-mdns-address-lookup/src/lib.rs#L374) |
| QNT 触发 | 本端地址变化、对端地址增删、重大网络变化触发探测；周期检查 60 秒，无 IP 路径或最小 IP RTT 大于 10ms 才升级；无新候选时轮次至少相隔 5 秒；[地址变化](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state.rs#L288)、[周期](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state.rs#L454)、[轮次](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state.rs#L753) |
| 探测接口 | noq 有公开 open_path，iroh 在私有字段内使用它，没有向其 Connection 公开远端地址探测入口。Endpoint.add_external_addr 宣告的是本机地址；[内部调用](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state.rs#L1051)、[noq](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq/src/connection.rs#L404) |

第三个临时程序使用 loopback、合成身份和一个真实可达的 UDP 转发地址。原连接已有 selected path 时，向尚存活的 lookup stream 推入该地址，remote_info 确认缓存更新；随后 500ms 内未见新候选收到包或成为 path。正对照由 peer 通过 add_external_addr 宣告同一地址，在 5 秒限时内观察到探测和 Opened；stable Connection ID 不变，前后已知载荷仍成功。该差异与源码没有 lookup→探测调用相符，但不证明长期不会由其他事件触发，也不验证 mDNS、Android 或真实 WAN→LAN 的选择质量。实验源码与布尔结果只留临时目录，不作为自动化测试或产品依赖。

实际 path watcher 有地址、选中状态与统计；当前 Velune paths 映射只保留 Direct／Relay 等，无法区分公网 Direct→LAN Direct。验收需关联连接内临时 path ID、来源、地址族、事件和字节增量；地址来源或私网范围不能单独证明业务实际走 LAN。[原生观察](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state/path_watcher.rs#L453)、[当前映射](../../packages/remote/src/paths.rs)。

## 现有 Velune 契约的缺口

`remote::State` 同时持有 iroh Endpoint、随机 incarnation、Ledger 和完成记账任务。新 bind 就是新世代和空账本；这与仍然存活的 ApplicationService 不同。普通断连接的既有验证不能覆盖销毁绑定 runtime 后的记账寿命。[现有实现](../../packages/remote/src/endpoint.rs)、[bindings 装配](../../packages/bindings/src/remote.rs)。

现有 Pair 验证 token 后立即持久 trust 并消费邀请，再回 Paired；客户端最后写自己的 trust。没有执行端待确认状态，客户端失败可能形成单端保存。同一 DeviceTrust 又同时作为出站身份 pin 与入站访问 allowlist，双端写入会扩大为反向授权。Android 当前没有 Handler，不代表双执行设备场景不存在该问题。

当前 Android 仅在网络回调调用 networkChange，前台恢复复用旧连接并轮询，没有完整自动重连。DeviceStore 持久化完整 descriptor；已有 generation 可防止部分晚到结果，但尚未形成统一的设备／配置连接意图。

## 设计判别点

人工恢复验收应分别控制路径、QUIC 连接、网络端点、远控服务与业务 owner 的寿命。每项观察至少关联当前界面、执行端实际副作用及原请求／连接身份；进程存在、网络通知返回或无错误日志不作为通过。

优先判别：延迟合成写入在网络端点重建后用原 ID 查询到同一次结果；真正重启 owner 后同 ID 不再执行；配对确认／断线／落盘竞争下只出现已提交可查询或未授权失效；离线控制端遇到整套 lookup 更换时得到可处理的连接资料缺失，而非误报未配对。

Android 默认网络与特定 Network 的 socket 路由不同。[官方网络状态说明](https://developer.android.com/develop/connectivity/network-ops/reading-network-state) 和 [逐 socket 绑定](https://developer.android.com/reference/android/net/Network#bindSocket(java.io.FileDescriptor)) 支持这个区别，不能从接口枚举或默认 callback 推导非默认 Wi-Fi 可达性。该问题仍需锁定 iroh 调用链与真机判别；本轮未改网络设置。

## Android 网络与扫码适配

iroh 重大网络变化处理实际执行 transport rebind、relay 检查、DNS reset 和重新探测；轻微变化仅探测。`Endpoint::network_change()` 返回只说明外部提示已发送。锁定 netwatch 0.19.3（`0030e57fc70981895bb86212ff0f059cc41c8a4e`）重新采样后，如果 State 相等会直接返回。[iroh 处理](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket.rs#L1657)、[netwatch 去重](https://github.com/n0-computer/net-tools/blob/0030e57fc70981895bb86212ff0f059cc41c8a4e/netwatch/src/netmon/actor.rs#L122)。

UDP 重绑先释放旧 socket，再用原地址／端口创建新 socket，不是每个 Android Network 创建一个 socket。[实现](https://github.com/n0-computer/net-tools/blob/0030e57fc70981895bb86212ff0f059cc41c8a4e/netwatch/src/udp.rs#L820)。Android RouteMonitor 为空，依赖 Java 提示；采样没有 Android Network 身份和 DNS，默认路由检测也没有使用 ConnectivityManager 的 active Network。[Android monitor](https://github.com/n0-computer/net-tools/blob/0030e57fc70981895bb86212ff0f059cc41c8a4e/netwatch/src/netmon/android.rs#L14)。因此 Android 默认网变化可能未被这份采样区分。

DNS reset 替换 resolver，在途查询重试；底层 n0-dns-resolver 0.1.0 保留 DNS cache、下次需要查询时重读系统配置。Android 配置来自 active Network 的 DNS，并非所有网络各自的 resolver。[iroh reset](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh-dns/src/dns.rs#L681)、[Android DNS reader](https://docs.rs/n0-dns-resolver/0.1.0/src/n0_dns_resolver/system_config/android.rs.html#71)。

Network.bindSocket 的 FileDescriptor 接口与 NDK `android_setsocknetwork` 在 API23 可用；当前 Velune／iroh／netwatch 链路没有使用。全进程绑定会同时改变未来 socket 和 DNS，不能作为无条件修复。[Android NDK 说明](https://developer.android.com/ndk/reference/group/networking#android_setsocknetwork)。后续应比较：Wi-Fi 真断开、Wi-Fi 保留但 Internet 失效、蜂窝默认而 Wi-Fi LAN 仍在、仅 DNS 改变。分别观察路径、实际往返与恢复时间，不由回调返回推断成功。

扫码建议为 CameraX 与 ZXing core；官方候选 CameraX 1.6.2、ZXing core 3.5.4 未写入本仓库依赖，仍需固定与构建。Google Code Scanner 依赖 Play Services，不适合作为本次华为设备的唯一入口。[CameraX 版本](https://developer.android.com/jetpack/androidx/releases/camera)、[ZXing POM](https://raw.githubusercontent.com/zxing/zxing/zxing-3.5.4/core/pom.xml)、[Google Code Scanner](https://developers.google.com/ml-kit/vision/barcode-scanning/code-scanner)。ImageAnalysis 使用最新帧背压并逐帧关闭，扫码页释放 camera；系统文件选择仅接触显式选中的图片 URI。[ImageAnalysis](https://developer.android.com/reference/androidx/camera/core/ImageAnalysis)、[文件选择器](https://developer.android.com/training/data-storage/shared/documents-files)。

mDNS 0.6.0 声明 iroh／iroh-base `1.0.0` 范围，具备与锁定 1.3.0 统一解析的元数据条件；本轮未编译或验证平台多播。[固定清单](https://github.com/n0-computer/iroh-address-lookups/blob/c64aec6e2afb0d0b3563858a58e70a00b8bfa94e/iroh-mdns-address-lookup/Cargo.toml)。

2026-10-18 S1 选型更新：官方 [修复提交](https://github.com/n0-computer/iroh-address-lookups/commit/6f103e7a92ddbd2007383527ab5024c983e3c1c6) `6f103e7a92ddbd2007383527ab5024c983e3c1c6` 将缓存改为 AddressLookupItem、比较实际 EndpointInfo 并覆盖插入；同一公钥地址更新后的 resolve 可取新值，重复时间戳不当作内容变化，过期仍删除。包版本仍为 0.6.0，iroh 依赖范围未变，当前固定该 git rev、保持默认发现周期。host／Android 编译、独立同 peer A→B／过期及 Host UniFFI 判别已通过，手机发现尚未通过，实际产物与边界归 [verification](verification.md)。早前发布版本的缓存风险不能套用为该修复提交仍有同一缺陷。

## mDNS 出口与寿命判别

2026-10-18，手机仅 ID 初连失败，但 mDNS enable 已成功。新版诊断观测手机 publish 收到 1 个 direct 地址、host 收到 2 个，随后仍未发现或连接；计数只证明 EndpointData 进入服务，不证明报文发出或收到。Mac 只读路由显示到手机的单播走 LAN，而 `224.0.0.251` 默认路由走既有 utun。本轮没有配置或修改隧道，不能宣称系统无 VPN，也不能由路由表单独认定报文路径。

已排除 late-add 漏发假设：iroh 1.3.0 `address_lookup.rs:495–512` 的 `add_boxed` 向新服务补发 `last_data`；Mac immediate／warm 两个临时实际调用均在 add 返回前观察一次非空 publish，并由独立 mDNS fresh resolve 取得地址。这个对照只覆盖 Mac，不能代替手机发现。

锁定 `swarm-discovery 0.6.4` 的 `socket.rs` 在没有显式 IPv4 接口时使用默认多播路由；已有逐接口 join／发送及动态 add/remove，但上层 mDNS wrapper 未暴露接口装配。同协议、同公钥和同载荷的“默认→默认＋显式 LAN”手机对照已取得：默认阶段新进程 12.290 秒失败、echo 为零；只增加 LAN 接口后新进程按 ID fresh resolve 并完成至少 18 次 Direct IPv4 marker。该差异支持接口装配缺口，不是仅看路由表的推断。原默认 receiver／membership 与自动 IPv6 仍保留，不能将其签为“仅 LAN”或严格退出，也没有改进程或系统路由。随后手机 A→B／过期与完整结果归 verification，不能以临时 fixture 本机自检替代。

advisor 对源码的寿命复核发现：`guardian.rs:119` 移除接口只丢 receiver 的 ActoRef，`receiver.rs:20` 仍持 socket 并等待接收；该返回不能证明资源已退出。条件性推荐是在稳定 mDNS lookup 内按实际 LAN 接口集合整体重建 discovery，空集合停用而非回退默认路由；保留 Endpoint、业务连接和外部订阅，并以代际排除旧 callback。实际接口退出仍需运行证据，DropGuard 发出 abort 不能替代关闭屏障。

显式 LAN 正对照已经触发官方 mDNS 单 crate 的最小本地补丁：保留固定来源／双许可、wire format 和缓存修复，不另造内部 discovery、不修改 iroh 传输栈。此前不 vendor 是官方修复已足够时的工程选择；新增接口证据改变此前提，改为可审计单 crate vendor，不引入上游整个 workspace、测试或 CI。当前补丁进入实施，尚未验收；IPv4 对照不能证明 IPv6 接口语义。用户已确认当前设备没有蜂窝、仅 Wi-Fi，真实 WAN→LAN 与非默认 Wi-Fi／蜂窝暂记环境阻塞。

vendor 临时进程的 empty→LAN→empty→LAN→drop 已观察到 5353 FD 数量 0→2→0→2→0，配置 ack 与实际关闭分别采样，只覆盖这份库生命周期。新增源码边界为：swarm 保留 wildcard socket 的默认接收 membership；非空显式 sender 全部创建失败时仍可默认发送。工程决定保留该回落和单 crate 范围：它提供 LAN 可达性装配，不提供接口流量隔离；禁止回落也不能修好创建失败的 LAN sender。空集合仍严格不 spawn discovery，创建失败诊断可见，配置代际不等于可达性证明。不增加第二份 swarm vendor 或无竞态保证的 preflight。

## 配对 exporter 样例与关闭屏障

第二个临时 Rust 程序仅绑定 loopback、禁用 relay／lookup，核验双方 Connection.remote_id 后构造同一 131 字节 context。label 为 `EXPORTER-Velune-Pairing-v2`（26 个 ASCII 字节，无 NUL）；context 编码与方案一致。输出仅布尔值和长度，不输出密钥、token、binding 或核对码。

| 观察 | 结果 |
| --- | --- |
| 双方真实身份核验、角色 context 相等 | true，context 长 131 字节 |
| 双端同 context 的 exporter 相等 | true，输出长 32 字节 |
| 原连接重复派生稳定 | true |
| 前八字节 BE 模 100000000 的显示 | 双端相同、八位 ASCII 数字 |
| 分别改角色、nonce、attempt、方向 | 双端仍一致，完整 binding 相对原值改变 |
| 第二条独立 Connection | stable_id 不同；双方结果一致，完整 binding 区别于第一连接 |
| 建立第二连接后重算第一连接 | 原 binding 稳定 |

改变 context／连接时，样例中的短码也改变，但不能保证八位短码没有碰撞。nonce／invite／attempt 是合成 fixture，不是已经实现的准入状态机；该实验不证明随机源、总挑战次数、并发授权或完整协议安全。[exporter API](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint/connection.rs#L1106)、[真实 remote_id](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint/connection.rs#L1159)。

Endpoint.close 本身清共享 lookup registry、关闭 QUIC、取消内部 actor，内部在途查询随 actor 释放；普通 Endpoint clone 不阻止该关闭状态。外部 PkarrPublisher clone 则可独立保活 spawn 任务，UDP FD 最终释放要求全部对应 Endpoint clone drop。[close](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket.rs#L1129)、[publisher 寿命](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/address_lookup/pkarr.rs#L270)、[FD 释放契约](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint.rs#L1725)。源码在已经 closing 时可提前返回，所以必须由唯一关闭 owner 等待一次完整关闭，不能使用第二次 close 作为退出完成证据。以上不代表可以撤回在途流量或远程历史记录。
