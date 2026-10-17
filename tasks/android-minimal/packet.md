# 最小 Android 远控端

2026-10-17 用户授权 Kotlin／Jetpack Compose 最小 Android 端，并允许使用已连接 adb 真机验收、实验。实现、静态检查、两端构建安装及 Mac 入口回归已完成；Android 真机业务验收等待用户手动解锁设备，尚未完成。

## 工作区与权限

用户随后要求迁移进行中的工作并将原目录恢复 main，已完成。本对话附属 `android-minimal` worktree 持有 `feat/android-minimal`；原目录当前为 main 且干净，没有覆盖其他任务更新。迁移检查点为 `3b8bed7`，功能提交为 `235b62b`；为保留当前已安装的并行修复，只在独立分支合入固定 main `3f01f9b`，最终源码为 `d3b039d`。原生库、生成绑定、交叉缓存和验收资源均已迁移，关键文件摘要核对一致。

本轮不 push、创建 PR 或部署公网服务，不读取真实配置、凭据、个人资料或会话，不调用真实模型；没有新增或运行自动化测试。adb 只操作本次 Velune、公开设备能力和合成资料。设备认证由本人完成，不尝试密码或绕过系统锁屏。

## 实施边界与当前证据

Android 是 remote-only UniFFI／iroh 原生控制端，不携带 application、Harness 或 AI 执行栈。最小界面包含显式配对／可信重连、路径状态、会话列表／历史、新建及运行时／模型选择、收发／取消、审批与问题回答。长期完整业务 SDK 范围不变，本轮不复制所有桌面配置界面、附件操作或复杂变更审阅。

Mac 设置提供同进程接入启停、五分钟一次邀请和设备撤销，复用已打开的唯一 ApplicationService。控制端后台／断开与远端执行寿命分开。Android 的未决写入先持久保存请求 ID，结果未知时查询原编号，不盲目重发；身份与信任放私有且不备份的目录。

- Android API28／arm64 的 native release、JNI／DNS／TLS verifier、实际 Kotlin／Compose／JNA 编译与 DEX 打包已完成；clean／assembleDebug／lintDebug 通过，lint 0 errors、7 条版本／ABI 范围提示。真实编译发现的 UniFFI close／List 命名冲突及低 API Cleaner 问题均从源契约／生成配置修复，没有 patch 生成代码。
- 全 workspace locked check／严格 clippy、fmt、Android target strict clippy、文档链接／锚点及人工脚本语法检查通过。remote-only 依赖树只含 bindings／application-api／conversation／remote。main 的 ProviderTemplate 归入中立 API，保留全部新字段，不恢复 Pi 私有公开元数据。
- 最终 Android APK 源码为 `d3b039d`，SHA-256 `3eacf0e86be715cd7dd0744d14978cc3b68c51dab64941fa65c2083c3deb75d4`，已 adb 安装并启动进程；限定 PID 日志未见 AndroidRuntime／JNA／UniFFI 错误。该观察不证明配对或业务成功。
- 同一源码 Mac release 从干净工作区构建，已完成全新合成 HOME 的真实入口启停／邀请／独立 AX 按钮和权限回归，安装至 `/Applications/Velune.app`。安装前核实原公开清单 `3f01f9b` 是构建祖先；签名、二进制摘要及清单 `d3b039d`／dirty=false 通过，版本 `0.1 beta.1`。未启动真实 app。
- 匹配 host／Python 的隔离提供商配置脚本通过，合成资料和显式秘密边界保持。各版本、命令、摘要和现场缺口归 [验收记录](verification.md)，不要把旧产物观察套用到新版本。

## 当前唯一前提与接续

真机是 API28／arm64、同 Wi‑Fi，但仍处于本人认证锁屏；已请求用户手动解锁并保持亮屏。未读取通知、截图锁屏或修改系统锁定设置。配对、实际 Direct／Relay、消息、审批、后台／断线接续及未知结果恢复的 Android UI 证据均未取得。

所有临时执行宿主和隔离 Mac 已正常退出，合成资料保留；手机没有配对或未决操作需要清理。用户解锁后，由原 verification owner 在独立工作区启动匹配最终产物的合成宿主、刷新邀请，继续真机闭环；必要问题交原源码 owner 修复。不能将本轮同 Wi‑Fi 实验当公网 NAT、真实切网或长期移动后台保证。

## 责任

android_app 拥有 `app/android` 与 APK；android_native 拥有 Android 原生构建／平台装配；android_mac_host 拥有 Mac 接入；remote_application 负责中立 API／application／bindings 的基线集成；android_verification 负责合成宿主及真机验收。主执行者维护根文档、分支与集成、隔离 Mac 验收安装和本地提交。各 owner 保持后续修复责任，原目录 main 不再用于本任务修改。
