# Android 控制端

此 unit 使用 Kotlin、Jetpack Compose 与 Material 3，经 remote-only UniFFI／iroh 连接执行设备。Android 不加载 application、Harness、网关或 Node 资源，也不把执行设备的路径解释为手机路径。当前支持 API 28 及以上的 arm64-v8a 设备，应用 ID 为 `app.velune.android`。

最小业务闭环包含一次性邀请配对、已配对设备重连、实际 Direct／Relay 路径、会话列表与历史、已有运行时／模型选择、新建会话、发送、停止及审批／问题回答。消息按用户右侧、LLM 左侧、系统／Harness 状态居中排列，不显示头像或昵称。正文可选择复制；工具和思考正文按需展开。附件当前只显示历史中的附件名称，不提供上传、预览或下载。提供商、资源配置、变更审阅和完整会话管理界面仍归后续产品迭代，不由此最小端隐式移除其远控契约。

## 构建与安装

开发机需要 JDK 17、Android SDK 36、Build Tools 35.0.0、NDK `30.0.14904198`、Rust `aarch64-linux-android` target。Gradle wrapper 固定为 8.13 并校验官方分发包 SHA256；Android Gradle Plugin 为 8.11.1，Kotlin 与 Compose compiler 同为 2.2.21，Compose BOM 为 2025.10.01。AGP 的 SDK／Gradle／JDK 约束见 [官方兼容说明](https://developer.android.com/build/releases/agp-8-11-0-release-notes)，Kotlin 2.x 使用 [Compose compiler 插件](https://developer.android.com/develop/ui/compose/compiler)。这些为已验证的固定构建组合，不追踪最新版。

在仓库根目录运行：

```sh
rustup target add aarch64-linux-android
scripts/build-android.sh
adb install -r app/android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n app.velune.android/.MainActivity
```

脚本先执行 [原生构建脚本](../../scripts/build-android-native.sh)，生成 `target/android-bindings` 的 Kotlin 契约、`target/android-native/arm64-v8a/libvelune_bindings.so` 和版本／哈希固定的 rustls Android verifier AAR，再运行 debug APK 构建与 Android lint。只改 UI 且原生契约未变化时，可使用 `scripts/build-android.sh --skip-native`；绑定必须与 so 配套。脚本不安装、不启动应用，也不读取用户配置。Gradle 与依赖首次构建需要公开网络。

`VeluneAndroid.onCreate` 在任何端点构造之前装载 so 并调用 `NativePlatform.initialize(applicationContext)`。该 JNI 入口提供 Android DNS 与系统证书验证所需的 JVM／Application context。只调用 JNA `Native.load` 不足以初始化这些平台能力。generated Kotlin 与 native library 都属于构建产物，不提交。

## 连接与恢复

Mac 的远程控制入口创建邀请后，用户在 Android 显式粘贴并配对。应用只在按下“粘贴邀请”时读取剪贴板。邀请和消息正文留在内存，不进入日志、持久配置或备份；邀请默认隐藏，可由用户显式显示／隐藏；输入关闭自动纠正与自动大写，避免键盘改写邀请。成功配对后清空邀请框并恢复隐藏。身份／可信设备文件位于应用的 `noBackupFilesDir`，应用同时禁用系统备份与设备转移。卸载或清除应用数据会丢失身份，需要重新配对。

当前端点显式选择 iroh 的公共开发 relay／discovery，正常采用 Automatic 选路；Direct 或 Relay 来自实际连接状态。开发设施没有 Velune 生产服务承诺。同 Wi-Fi 真机验收不能证明跨公网 NAT、移动网络切换或长期后台稳定性。

UI 状态由 ViewModel 管理，FFI／网络调用运行于 IO 调度器。进入后台仅暂停业务及路径观察；执行设备继续工作。`NetworkMonitor` 分开观察默认网络与可用 LAN；LAN 请求接受 Wi-Fi 或 Ethernet，不要求 Internet／validated，因此蜂窝或 VPN 成为默认网络时仍可报告底层 LAN。[Android 的 transport 请求按备选集合匹配](https://developer.android.com/reference/android/net/NetworkRequest.Builder#addTransportType(int))。Wi-Fi、cellular、Ethernet 和 VPN 字段均来自实际 capabilities，后台不修改这些事实。可用、丢失、链路属性和能力回调以 100 毫秒有界合并窗口发送具名网络快照，前台恢复也报告当前事实。`reportNetworkChange` 的序号仅确认通知已接收；实际路径由独立 `observePaths` 观察，界面显示连接编号和路径编号，不把通知成功当作网络恢复。没有前台常驻 service。应用重开后用户从已配对设备列表重连；“移除此记录”只移除本机拨号记录，执行端授权需要在执行设备撤销。

网络快照中的接口地址与 DNS 为纯 numeric IP；Android IPv6 的 `%scope` 后缀从 IP 文本去除，scope 归属由同条记录的 Network ID／interfaceName 表达。Java `hostAddress` 的非空契约在转换边界明确检查，不静默丢弃地址。这些事实仅用于观察，不用于构造连接 socket，不能把去 scope 的链路本地地址拿去拨号。

写操作先把请求编号、所属设备、操作类型与目标保存到私有文件，再提交封闭的具名命令。同一时间只跟踪一个未决写操作；连接丢失后“查询结果”查询原编号，不重新发消息。未知、回执过期、执行端重启都保留待确认状态。用户检查执行结果后可明确“结束查询”；这不会取消执行端的工作。持久文件不保存命令正文。控制连接释放和断开均不调用停止执行；停止必须使用会话里的“停止”操作。

## S1 合成传输探针

`TransportProbeActivity` 仅在 debug 源集注册，使用 `remote-diagnostics` 原生能力、独立 echo ALPN 和临时密钥；它不加载产品身份、可信设备、邀请或业务 Handler。原生构建必须包含该 feature 后才能构建 debug APK。正常产品界面不引用探针 API，release 源集不包含此 Activity、多播权限或平台辅助类。Gradle main 仍读取共享 `target/android-bindings`／`target/android-native`；使用诊断产物进行 release Kotlin 编译仅证明源码隔离，不证明实际 release APK 已剥离诊断 native exports。本轮交付 debug S1 探针包。

验收 owner 将本轮合成目标写入应用私有 `files/s1-probe/target.json`，再显式启动 `app.velune.android/.TransportProbeActivity`。目标结构为：

```json
{
  "endpointId": "64 位十六进制公开端点 ID",
  "relayUrls": ["https://受控中继"],
  "directAddresses": ["受控 IP:端口"],
  "publicInfrastructure": true,
  "intervalMs": 1000,
  "payloadBytes": 256,
  "timeoutMs": 2000
}
```

`publicInfrastructure=false` 时本端禁用 relay 和 discovery，且要求目标的 `relayUrls` 为空，仅使用目标给定的可直达线索；不符合的 fixture 明确启动失败。`mdns` 可选字段默认 `false`，默认探针不启动 mDNS。文件不接受凭据型 relay URL；端点 ID、列表长度、控制字符和低流量参数在进入原生前校验，具体地址格式由原生边界解析。marker 间隔允许 1–10 秒、载荷 1–1024 字节、单次超时 100–10000 毫秒；默认值如上。

独立 mDNS case 使用 `mdns=true`、`publicInfrastructure=false`，且 `relayUrls` 和 `directAddresses` 均为空，仅给受控端点 ID。此分支显式启用 mDNS，非引用计数的 `WifiManager.MulticastLock` 仅在前台、请求 mDNS 且有实际非 VPN 的 Wi-Fi IPv4 发现需求时持有；Ethernet-only 不需要 Wi-Fi 锁。后台、网络需求消失或结束即释放，恢复条件满足后重新持有同一 probe 的锁，不重新拨号。debug manifest 声明 normal 级 `CHANGE_WIFI_MULTICAST_STATE` 权限，不弹运行时授权；取得或释放失败有独立失败状态与私有记录。[Android 官方说明](https://developer.android.com/reference/android/net/wifi/WifiManager.MulticastLock)指出多播锁允许 Wi-Fi 接收多播包，使用完应释放，且不影响其他应用持有的锁。

mDNS 事件每 500 毫秒排空一次，只把当前受控目标 ID 的 Discovered／Expired、本端 Published 的地址数量、InterfacesApplied 的代际及 IPv4 列表和本 ring 的 Dropped 计数落盘。Published 仅证明 mDNS service 收到 EndpointData，不证明数据已发出、Android 权限正确或对端已收到。InterfacesApplied 表示 actor 已接受接口集合并向旧 guard 发出 abort 请求，不证明旧 FD 已退出；空接口集合禁用发现器，不回落默认接口或 IPv6。Expired 也可能由本地接口代际失效产生，不能据此判定远端离线。`Dropped.count` 为自上次 drain 以来新增的丢弃数，随本次读取消费，不是累计值，不需要历史去重。连接成功后显式查询一次当前 mDNS cache，前台期间另每 2 秒查询，并在目标 Discovered／Expired 后查询；这些查询不创建连接。查询结果地址只进入私有证据，未发现通知不能代替查询结果，未收到通知也不推断未更新。上游订阅队列没有完整漏事件信号，本地 Dropped 只说明本地 ring 溢出；不能把它为零当作完整上游事件历史证明。此模式用于独立发现／缓存判别，不用于补强默认 QNT case。当前手机无 Ethernet 实测，监听和字段接线仅有静态证据。

每个 ViewModel 只调用一次 `connect`。网络变化、marker 超时、后台恢复和路径丢失均不会拨第二条连接；旋转保留该 ViewModel。后台暂停 marker 与路径／原生事件读取，前台沿用原连接；窗口从连接成功起最多 300 秒，或由“停止探针”结束，结束时关闭本轮资源。重开已销毁的 Activity 才开始新的独立运行，不能把它算作原连接迁移。

`files/s1-probe/events.jsonl` 以私有权限记录原始 Android 回调时刻、合并通知回执、marker 序号／校验结果／时延、具名连接／路径统计、受控路径地址及原生白名单事件。每行包含独立运行 ID、事件序号、Android 单调时间；原生采样时间保留原字段，不能直接当作 Android 时间原点。`path_observed` 保存原生 `missedEvents`；非零表示当前快照已补齐，但之前的路径事件历史有缺口，探针停止并标为证据不完整。原生白名单事件每 500 毫秒读取，丢弃计数或落盘失败也停止并标为证据不完整。日志写入在 IO 调度器经有界队列进行，地址、目标、载荷和事件内容均不进入 Logcat 或页面。探针结束只代表观察窗口完成；10／15 秒预算、路径证据和真实网络条件仍按任务验收方案分别判断。

诊断文件可写时，启动失败以 `probe_failure` 记录固定 `stage` 和 `errorKind`，mDNS 查询／多播锁失败也携带这两个字段。目标读取、JSON／目标解析、端点创建、mDNS 启用与 connect 分开；connect 失败不再称为 mDNS 初始化失败。错误种类只取穷尽的 UniFFI variant、受限平台异常类型及有界 `BindingFailureKind` 枚举，其余为 `unknown`。不解析或保存异常 message、detail、动态 code／phase、cause 或 stack；只有实际超时异常类型才标为 `timeout`，`Diagnostic(CONNECTION)` 不能据耗时推断为超时。界面根据实际阶段使用固定操作提示。
