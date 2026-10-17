# 最小 Android 远控端

2026-10-17 用户要求实现最小 Android 端，建议 Kotlin／Jetpack Compose，并授权使用已连接 adb 真机验收、实验。从远控 SDK 分支的 `b4e5b2d` 开始，在 `feat/android-minimal` 实施；本轮不 push、创建 PR 或部署公网服务。

## 目标与边界

交付可安装的原生 Compose 控制端，经现有 remote-only UniFFI／iroh 操作执行设备，不携带 Harness 或 AI 执行栈。最小闭环包括显式邀请配对、可信设备重连、会话列表与阅读、已有运行时／模型选择及消息收发、执行状态与审批；不以网络回声或纯只读演示代替。完整远控的长期业务范围不变，本轮不复制桌面所有配置界面。

执行端共用既有唯一 ApplicationService；advisor 已推荐补齐 Mac 的最小远控设置入口，因为独立诊断宿主不能替代用户可以开启的产品执行端。Mac owner 正在贯通启用／关闭、邀请和设备撤销，默认显式选择公共开发设施并说明性质。合成隔离宿主用于开发验收，不替代产品执行端。控制设备后台／断开不停止执行端；结果未知不自动重发。

不得读取真实配置、认证、个人文件或会话，不调用真实模型。adb 只操作本次安装的 Velune、公开设备能力与本任务合成内容；不清理其他应用或数据。使用静态检查、构建、人工脚本与真机操作，不新增或运行自动化测试。必要 Mac 改动按规范重建安装，用户已授权退出旧 Velune。

## 当前事实与责任

工作区开始干净。已连接单台 Android 设备，API 28、支持 arm64-v8a，1080×2340；同 Wi‑Fi 可提供本地真实网络证据，不推定公网直连稳定性。开发机有 Android SDK／NDK、Android Studio JBR 和 JDK17，目前没有 Gradle 命令或 Android Rust target。

- android_app owner：`app/android` 工程、Compose、Gradle 与 APK 构建；负责自身编译修复。
- android_native owner：remote-only Android so／Kotlin 生成与 `scripts/build-android-native.sh`，必要 Rust 平台修复。
- advisor：最小产品执行端接线和生命周期判断，不作 reviewer。
- android_mac_host owner：Mac 同进程远控入口与原生设置。
- android_verification owner：合成宿主及 adb 真机验收、当前独占设备 UI 操作。
- 主执行者：根文档／忽略规则、集成、隔离 Mac UI 验收、必要 Mac 安装和本地提交。

## 验收与下一步

先闭合 Android API28 的原生装载、Kotlin/JNA 编译、设备可达路径与执行端邀请。随后在临时 HOME／合成 ACP 宿主中通过真机界面完成配对、会话、发送、审批和断线接续，观察实际 Direct／Relay。至少检查应用重开后的可信重连、后台不取消任务以及结果未知的防重复行为。记录能力未覆盖项及现场边界；构建成功不代替 UI 业务验收。

Android remote-only 原生库已完成 release、严格 clippy 与 JVM／Application JNI、DNS、系统 TLS verifier companion AAR 的装配；不把编译成功当网络运行成功。独立合成 LAN 宿主和供 Mac 独占打开的合成配置已分别准备，没有共开配置根；仅有公开开发 relay／discovery 网络，不含真实模型或资料。

用户随后要求将进行中的工作迁到独立工作区，并将原目录切回 main，现已完成。进行中源码保存于迁移检查点 `3b8bed7`，本对话所附 `android-minimal` worktree 持有 `feat/android-minimal`；原目录已切回其当前 main 且工作区干净，未覆盖其他任务更新。原生 Android so、生成绑定、交叉缓存、Gradle 中间产物及人工宿主资源均已复制，必要动态库摘要一致；仅删除了原目录已逐文件验证迁移成功的 Android 生成文件。合成宿主已关闭，恢复时只在新工作区继续。迁移检查点时尚未产出 APK；迁移后 Kotlin／Compose／JNA 编译和 APK 打包已完成。首版已通过 adb 安装及进程启动，但设备安全锁屏阻挡业务界面，已请求用户手动解锁，不能代替本人认证。Mac 本轮已有完整构建，最终修复版仍待干净构建安装。

真实 Kotlin 编译发现 UniFFI 的 Endpoint.close 与自动 Disposable.close 冲突，以及 BindingApplicationCommand.List 遮蔽 Kotlin List 泛型，已从源契约 hard-cutoff 改为 shutdown／ListConfiguration 并同步调用方。lint 又发现默认 JVM Cleaner 需要 API33，已采用上游 android=true 生成配置，使低 API 使用 JNA Cleaner；不 patch 生成文件。新配置已实际完成 Kotlin／DEX 编译，最终 clean／assembleDebug／lintDebug 全部通过（0 errors，7 条版本／ABI 范围提示）。Mac 合成界面启用失败的根因是宿主未创建私有 remote 父目录，源码已修复；补齐相同目录前置后旧隔离 binary 已成功启用，最终新 binary 仍须从全新 HOME 回归。状态行的停止按钮 AX 分组也已修复，待最终构建验证。

主执行者已完成新工作区的全 workspace locked check、fmt、文档链接／锚点与 Python 语法检查，严格 clippy 全部通过。当前先提交稳定源码并完成必要 Mac release 构建安装；Android 业务验收仍等待设备解锁。未将已安装、进程存活或独立 host 的成功解释为真机业务通过。

最终 APK 已重新安装并启动，SHA-256 `51667f5ac72e7174eb3e197bbe4aeb307a6ff2b0eb83339037ea9fa7c68e61e5`，没有观察到 AndroidRuntime／JNA／UniFFI 错误；设备仍安全锁屏。文件写入暂缓期间由 verification owner 保留该观察，待 Mac 构建清单生成后归入验收记录。
