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

原生库交叉编译已进入 iroh／bindings，发现 Android DNS 需要 Application JNI context、系统 TLS verifier 需要 JVM 初始化与 companion AAR；由 native owner 完整接线，不把编译成功当运行成功。独立合成 LAN 宿主和供 Mac 独占打开的合成配置已分别准备，没有共开配置根；仅有公开开发 relay／discovery 网络，不含真实模型或资料。

用户随后要求将进行中的工作迁到独立工作区，并将原目录切回 main。所有源码 owner 已暂停，合成宿主已正常关闭，Android 尚未产出 APK 或操作真机 UI。当前提交仅为迁移检查点，不表示 Android 交付或验收完成；原生 Android so／生成绑定及人工宿主资源一并保留后在新工作区继续。Mac 本轮完整构建已完成，但尚未进行隔离界面验收或安装。
