# velune-remote

通过 iroh 1.3.0 的端到端加密 QUIC 连接，将 `application-api` 的具名命令交给执行宿主唯一的 `Handler`。本包不依赖 application、Harness 或 LLM 执行实现，也不读取 `HOME`、`VELUNE_HOME` 或现有认证配置。

宿主显式装配 `DeviceIdentity`、`DeviceTrust`、`EndpointConfig` 和可选业务处理器。执行宿主提供处理器；只控制其它设备的 app 可以不提供。GUI 与 headless 使用相同入口。本地 UI 与网络处理器必须共享同一个 application owner，不能为每条连接重新打开配置根。

## 连接与公网设施

`RemoteEndpoint::bind` 创建端点；`descriptor()` 返回公开拨号信息，不能据此取得业务权限。默认不启用公共 relay 或地址发现。`RelayConfig::Custom` 接受自部署 relay 地址；`DiscoveryConfig::CustomPkarr` 使用自部署 pkarr 服务发布、查询设备地址。持有最新 descriptor 时也可以不启用发现；设备换网或 relay 地址改变后，长期保存的拨号信息需要地址发现服务更新。

`DevelopmentPublic` 是对 n0 公共开发设施的显式选择，不是 Velune 正式服务承诺。正式产品可由宿主装配开箱即用的服务地址，同时允许用户替换为自部署设施；本包不部署服务。地址不接受用户名、密码、query 或 fragment，避免把认证材料混入公开拨号信息。生产服务应使用 HTTPS。

默认 `PathPolicy::Automatic` 保留 iroh 的 IPv4／IPv6、relay 协助建连、直连探测与路径选择。relay 不是固定的数据路径；建立直连后由 iroh 选择实际发送路径。`RelayOnly` 清除直接 IP 传输，只用于受控诊断。`network_change().await` 转发原生网络变更通知，供平台前后台和切网事件使用，不另建网络监控器。

`RemoteClient::connection_status()` 返回当前路径及选中状态；`observe_paths().next().await` 先返回快照，随后按原生路径事件更新。事件积压时重新读取完整快照。统计只包含路径类别、RTT、字节数、丢包及公开设备身份，不包含网络地址或业务正文。每条业务请求使用独立 QUIC 双向流；大附件读取不会占住同一连接唯一的业务流。

## 配对、持久信任与撤销

执行设备调用 `create_invitation`，把最长十分钟、仅可消费一次的邀请交给另一设备。邀请包含秘密，只有用户主动配对时才编码或传递，不应记录日志；邀请、私钥和业务请求均不实现可打印正文的 `Debug`。客户端通过 `pair` 取得双方持久信任；之后通过 `connect` 重连。连接始终校验端点公钥，名称只用于显示。

`DeviceIdentity::load_or_create` 只访问显式传入的身份文件。首次写入原子发布，既有无效身份不重置，避免替换设备身份。`FileTrustStore` 以原子替换保存可信设备；Unix 文件权限为 `0600`，已有开放权限或非普通文件直接拒绝。宿主为每个信任文件共享一个 store 实例；其它平台可实现 `DeviceTrust`，使用自己的私有存储。`MemoryTrustStore` 用于合成验收或确实不需要跨启动信任的宿主。

配对保存成功后才返回接纳。断线若发生在接纳与回执之间，执行设备可能已经信任控制设备，但邀请已经消费；此时由执行设备检查可信列表并重新生成邀请，不自动重复配对。信任存储失败会明确报错。

应通过端点 `revoke` 撤销设备：先持久保存，再立即关闭该设备的所有现有连接；后续每条请求仍检查授权。已经接纳的业务操作归 execution owner 负责，不因撤销连接而隐式回滚或取消。

## 接纳、结果查询与断连

需要在发送前保留标识的调用使用 `next_request_id`、`submit`、`query`。`call` 提供等待结果的便利入口；不确定失败的 `CallError` 保留请求标识，调用方应查询该标识，不能为同一副作用另分配标识重发。网络入口、连接和结果等待均有明确超时；超时不等于业务失败。

标识由执行端 incarnation、客户端 namespace 和单调序号组成。相同标识和内容最多接纳一次，内容不同返回冲突。`ClientSession` 可通过 `session()` 和 `connect_with_session` 在重连时复用；同一 session 支持 256 个序号以内的乱序到达，`call` 不在网络上传或等待结果期间持有提交锁，允许审批、取消与附件并发。超出窗口的旧序号返回结果不可用，不重新执行。

结果保留在有界内存中。缓存回收后仍保留已接纳集合与退休水位，返回 `ResultExpired`，不会重新执行旧请求；这表示结果已不可用，也可能是窗口以外的未提交空隙，不应宣称一定完成过。`Unknown` 表示此 incarnation 尚无接纳证据。执行端重启后，可信设备可以连接到同一公钥的新 incarnation 并发起新业务；查询或重发旧 incarnation 的标识返回 `ServerRestarted`，不承诺跨崩溃 exactly-once。

客户端退出、掉线或端点关闭只结束网络访问；接纳后的处理任务独立持有业务 owner。持续推进 Harness、审批、发送队列和协作由 application owner 保证，本包不调用应用 shutdown。

## 当前边界与验证

| 边界 | 限额 |
| --- | --- |
| 已授权单帧 | 32 MiB；20 MiB 内容经契约 base64 编码后可以传输 |
| 未授权配对帧 | 16 KiB |
| 在建／在用入站连接 | 64；端点注册连接共 128 |
| 同连接请求流／全局请求处理 | 8／8 |
| 已接纳未完成请求 | 64 |
| 结果缓存 | 256 条，合计 32 MiB，先达到者回收 |
| 请求 namespace | 每次执行端启动最多 4096 个，重连应复用 `ClientSession` |
| 可信设备／待用邀请 | 256／16 |

调用、帧、版本、邀请、存储与授权错误使用无正文的 `RemoteError`；显式业务错误仍由 application 契约返回。完整业务写入冲突、陈旧审批、附件内容边界与每设备独立视图由执行 owner 负责，不由传输 mutex 代替。

2026-10-17 已使用合成身份和处理器，经真实 iroh loopback 连接验证选中 Direct、配对、断连后继续执行、重连查询、同标识去重、冲突与撤销；显式公共开发 relay 的受控 RelayOnly 连接验证选中 Relay 并通过相同闭环。两者都不是跨 NAT 直连、真实切网或移动后台寿命的证明。静态检查与完整应用闭环见[任务验证记录](../../tasks/remote-control/verification.md)。本包没有自动化测试或 CI 测试入口。
