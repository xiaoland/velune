# 平台接入

[返回远控契约](remote-access.md) · [产品行为](../prd/remote-access.md)

Rust 提供本地批准 API、网络事实输入、活动意图与具名状态；平台负责相机、二维码显示、系统权限页面与 App 生命周期接线。

Android 已实现的装配为 Kotlin／Compose → UniFFI 生成的 Kotlin／JNA API → `libvelune_bindings.so` → Rust remote／iroh。构建仅启用 bindings 的 `remote` feature，不携带本地 application、Harness 或 AI 执行栈。平台初始化的少量 JNI 只提供 Android context，业务仍经 UniFFI 类型接口；新配对、恢复和路径策略继续归 Rust，Kotlin 提供界面、平台网络通知及相机生命周期。

Android 使用独立的联系人／待确认记录和入站授权概念，不能继续把完整 descriptor 当作永久设备记录。控制端只需恢复原请求 ID，不持久化消息正文来重发。扫码采用 CameraX 生命周期组件与 ZXing core 的离线解码路径，不强依赖 Google Play Services；候选版本及构建仍需实施时固定。相机仅在扫码界面持有；每帧关闭 ImageProxy、只处理最新帧，识别成功就停止分析。权限拒绝或不可用时，可用系统文件选择器选一张二维码图片，或显式粘贴邀请内容，全部进入同一解析和确认流程，不索取全盘媒体权限。

Android 默认网络回调不覆盖所有可用接口。无 Internet Wi-Fi 与蜂窝并存时，默认网络可能是蜂窝；不能用 `VALIDATED` 判定局域网可达性，也不能直接假定未绑定 socket 会走非默认 Wi-Fi。全进程绑定 Wi-Fi 又可能破坏公网 relay／lookup。先核实 iroh 的 socket 重绑与平台行为，再以受控真机实验决定是否需要逐 socket 适配，不在 UI 增加选网模式来掩盖问题。

锁定源码已确认重大变化会重绑 UDP socket 并 reset DNS；但 `network_change()` 只是提示，netwatch 采样相等会跳过。Android 适配没有 Network 身份／逐 socket 绑定，resolver 使用默认 Network 的 DNS。因此须把 Wi-Fi→蜂窝的恢复与两网同时可达分别验证。平台通知至少覆盖默认网络及链路属性变化；新增局域网监听和多播资源随前台需要管理，不以后台常驻、前台 service 或进程级强制绑网替代验证。

第一项实施判别先保留现有传输与 selector，仅补网络事件和路径观察，验证平台地址变化／QNT。随后分别比较非默认 Wi-Fi 上普通 UDP 与显式 Network 绑定：普通可达则不增加适配；只有显式绑定可达时，才在 UDP 创建及每次 rebind 贯通 Network 生命周期。iroh 1.3 没有内置 UDP FD 绑定回调，该分支需要经固定版本验证的窄补丁或上游扩展，不采用不稳定 custom transport 另造 IP 栈。两种 UDP 都不可达时先修实验环境，不推断库缺陷。

mDNS 单独验证初连、地址更新和过期。S1 选择官方修复提交 `6f103e7a92ddbd2007383527ab5024c983e3c1c6`（包版本 0.6.0），覆盖插入缓存；host／Android 编译和独立缓存更新／过期实验已完成。随后手机同公钥／同广告正对照确认，默认多播路由下发现失败，增加显式 LAN 接口后按 ID 发现并完成 marker。该事实触发有界依赖调整：只 vendor 此官方 crate 并维护接口装配的最小本地修改，保留来源、双许可、wire format、缓存修复和默认发现周期，不引入整个上游 workspace。源码证据、对照条件与后续缺口归研究记录和验收记录。

发现接口依据本机实际 LAN 链路选择，覆盖非默认 Wi-Fi／以太网，不由默认路由、Internet 验证标记、RFC1918 范围或 Endpoint 对外候选推导。复用平台网络事实与现有 netwatch，只补缺失的接口属性。有效接口集合变化只替换内部 discovery；Endpoint、业务连接、lookup 注册及外部订阅保持，空集合停用该发现来源，不能退回未经选择的默认多播出口。旧实例 callback 按发现代际隔离，缓存失效和订阅仍需明确处理。swarm 动态移除接口与 DropGuard 的返回均不能替代实际资源退出证据，补丁和两平台生命周期仍按 S1 验收；IPv4 通过不推导 IPv6 通过。

非空集合指定尝试装配的 LAN 发送接口，保留 swarm 的 wildcard 默认接收及全部显式 sender 创建失败时的默认发送回落，不保证报文仅通过这些接口。创建失败的既有诊断保持可见；不为流量隔离扩大到第二份 swarm 补丁。接口配置代际已更新只说明处理了该配置，不能显示为 LAN 已就绪；实际发现、marker 和路径证据分别验证可达性。

subscribe→MemoryLookup 不能替代遗漏的更新或活动连接探测。当前证据支持修正同协议的接口装配，不支持未经协议核对换成 Android NSD 或另造 discovery。真正跨网和非默认 Wi-Fi／蜂窝路由继续独立判别。



Android socket 接入点、QNT 通知遗漏与 mDNS 修复方式须以证据判别；真实 NAT、跨公网迁移和非默认移动网络行为不能由 loopback 或库构建推导。
