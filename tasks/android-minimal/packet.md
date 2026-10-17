# 最小 Android 远控端

2026-10-17 用户授权 Kotlin／Jetpack Compose 最小 Android 端，并允许使用已连接 adb 真机验收、实验。用户随后要求暂停实现与验收，重新讨论公共设施及配对体验；现已确认可用 iroh 公共设施、兼容 relay 可替换，以及扫码确认和统一局域网／公网体验。用户进一步授权继续深入方案设计，要求核实实际技术复杂度；产品实现与真机验收仍暂停。

## 工作区与权限

用户随后要求迁移进行中的工作并将原目录恢复 main，已完成。本对话附属 `android-minimal` worktree 持有 `feat/android-minimal`；原目录当前为 main 且干净，没有覆盖其他任务更新。迁移检查点为 `3b8bed7`，功能提交为 `235b62b`；为保留当前已安装的并行修复，只在独立分支合入固定 main `3f01f9b`，main 集成源码为 `d3b039d`。原生库、生成绑定、交叉缓存和验收资源均已迁移，关键文件摘要核对一致。

本轮不 push、创建 PR 或部署公网服务，不读取真实配置、凭据、个人资料或会话，不调用真实模型；没有新增或运行自动化测试。adb 只操作本次 Velune、公开设备能力和合成资料。设备认证由本人完成，不尝试密码或绕过系统锁屏。

## 实施边界与当前证据

Android 是 remote-only UniFFI／iroh 原生控制端，不携带 application、Harness 或 AI 执行栈。最小界面包含显式配对／可信重连、路径状态、会话列表／历史、新建及运行时／模型选择、收发／取消、审批与问题回答。长期完整业务 SDK 范围不变，本轮不复制所有桌面配置界面、附件操作或复杂变更审阅。

Mac 设置提供同进程接入启停、五分钟一次邀请和设备撤销，复用已打开的唯一 ApplicationService。控制端后台／断开与远端执行寿命分开。Android 的未决写入先持久保存请求 ID，结果未知时查询原编号，不盲目重发；身份与信任放私有且不备份的目录。

- Android API28／arm64 的 native release、JNI／DNS／TLS verifier、实际 Kotlin／Compose／JNA 编译与 DEX 打包已完成；clean／assembleDebug／lintDebug 通过，lint 0 errors、7 条版本／ABI 范围提示。真实编译发现的 UniFFI close／List 命名冲突及低 API Cleaner 问题均从源契约／生成配置修复，没有 patch 生成代码。
- 全 workspace locked check／严格 clippy、fmt、Android target strict clippy、文档链接／锚点及人工脚本语法检查通过。remote-only 依赖树只含 bindings／application-api／conversation／remote。main 的 ProviderTemplate 归入中立 API，保留全部新字段，不恢复 Pi 私有公开元数据。
- main 集成时 Android APK 源码为 `d3b039d`，SHA-256 `3eacf0e86be715cd7dd0744d14978cc3b68c51dab64941fa65c2083c3deb75d4`，已 adb 安装并启动进程；限定 PID 日志未见 AndroidRuntime／JNA／UniFFI 错误。该历史观察不证明后续身份初始化、配对或业务成功。
- 同一源码 Mac release 从干净工作区构建，已完成全新合成 HOME 的真实入口启停／邀请／独立 AX 按钮和权限回归，安装至 `/Applications/Velune.app`。安装前核实原公开清单 `3f01f9b` 是构建祖先；签名、二进制摘要及清单 `d3b039d`／dirty=false 通过，版本 `0.1 beta.1`。未启动真实 app。
- 匹配 host／Python 的隔离提供商配置脚本通过，合成资料和显式秘密边界保持。各版本、命令、摘要和现场缺口归 [验收记录](verification.md)，不要把旧产物观察套用到新版本。

## 当前真机边界与接续

暂停时，独立合成 host 已正常关闭 endpoint／ApplicationService，隔离 Mac 实例已退出，手机尚未配对成功且数据保留。Android 安装产物为 `33cb77b`（已加入邀请显示／隐藏与禁用自动纠正）；对应 UI 与合成宿主源码改动尚未提交，验收状态已在文档归位。最小身份发布修复已本地提交为 `0004d53`，未 push。外部设施和统一交互决定已归 [PRD](../../docs/prd/index.md#后续扩展分布式能力)，拟议接入调整归 [架构](../../docs/design/architecture.md#远控契约与唯一执行-owner2026-10-17)；扫码后的执行端确认、可刷新的设备地址及设施切换仍未实现。

真机是 API28／arm64、同 Wi‑Fi，用户现已手动解锁。首次真实 UI 显示身份存储错误；合成 marker 判别观察确认私有目录可创建0600文件，但同目录 hard_link 被拒绝。Android 专用 rustix `renameat2(RENAME_NOREPLACE)` 已实施，保留完整发布、不同写入者不覆盖与 fsync；marker 的无目标／既有目标／并发单赢家／重开完整检查通过，最终 so 无 API30 renameat2 动态符号依赖。

修复 APK `bd4c918` 已覆盖安装，身份文件实际发布为0600。随后“重开仍错”的观察已撤回：验收驱动误将 uiautomator busy 后遗留的旧 XML 当作新结果，现已改为先删旧文件并验证新 dump 成功。临时诊断已删除，最终 so 恢复为与该修复包逐字节一致；新鲜快照确认初始化成功。`run-as` 实验仍不替代实际应用进程证据。配对等后续 UI 证据仍待取得；该轮 workspace fmt、locked check／严格 clippy、Android target check／严格 clippy／release，以及 APK assemble／lint 均通过。

源码与本机实验已确认部分 relay／lookup 可热更新，但严格移除旧服务、改变 transport 或 token 可能需要网络端点重建。网络路径变化不改变设备信任、会话和审批，也不触发业务重发。当前不继续设备操作；不能将同 Wi‑Fi 实验当公网 NAT、真实切网或长期移动后台保证。

## 责任

android_app 拥有 `app/android` 与 APK；android_native 拥有 Android 原生构建／平台装配；android_mac_host 拥有 Mac 接入；remote_application 负责中立 API／application／bindings 的基线集成；android_verification 负责合成宿主及真机验收。主执行者维护根文档、分支与集成、隔离 Mac 验收安装和本地提交。各 owner 保持后续修复责任，原目录 main 不再用于本任务修改。

## 当前设计复核

native、Mac、Android、advisor 与 verification 的只读研究已整合为 [方案草案](../../docs/design/remote-access.md)，锁定 SDK 源码、本机判别实验和平台约束归 [研究证据](transport-research.md)。没有恢复产品源码改造、构建安装或设备操作。

关键拟议调整为：调用世代／有界账本／异步 runtime 与唯一业务 owner 同寿命；设备 pin 与入站 grant 分开；短时邀请仅开放候选请求，由执行端核对连接绑定数字再原子授权；联系人与 locator 分开；局域网和公网 lookup 同时装配，严格设施移除采用网络代际切换。源代码尚未实现这些保证。

草案已固定核对码的角色／字节上下文和确认提交顺序；独立 loopback 实验验证 exporter 双端一致、同连接稳定及新连接改变绑定，不证明完整配对安全。后续按四个切片：服务寿命和账本、配对状态机、locator／设施交接、两端扫码与恢复；需设计讨论后再恢复实现。Android 默认网络变化可能被 netwatch 采样去重，无 Internet Wi-Fi＋蜂窝并存的真实路由仍需判别；整套 lookup 撤离后的离线端需带外更新 locator，不重新建立控制授权。

用户新增已建立连接的多路径自动优选要求：公网进入同 LAN 后自动发现更优路径，离开后恢复。native owner 的锁定源码与 loopback 对照确认：晚 lookup 只入缓存，QNT 广告相同候选才打开原连接新路径；默认 IP 优先、RTT／迟滞选择，不持续综合丢包或带宽，且可能关闭冗余 IP 路径。advisor 建议活动升级沿本机地址变化／QNT 主线，mDNS 补初连重连，不先建立第二条业务连接；仅当平台验收出现可复现遗漏时评估上游探测接口。研究已收敛，剩余判据为真实 Android／Mac 进入与离开 LAN、非默认 Wi-Fi／蜂窝路由，以及路径与业务连续性。

已重新核对本地 APK `33cb77b` 确含 Rust so `2529e914`，构建脚本采用 remote-only UniFFI；无需重装或读取设备数据证明这一装配事实。本轮只更新设计及状态文档；产品源码、构建安装和真机验收继续暂停，恢复实施时先完成上述四个切片的核心契约。

本轮七份设计／状态文档的 209 个本地链接及标题锚点检查通过，git diff／cached diff whitespace 检查通过；原目录仍为干净 main。没有因文档改动重跑产品编译，也没有新增或执行自动化测试。
