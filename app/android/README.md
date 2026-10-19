# Android 控制端

Android 是只控制、不执行的端。它不运行 Agent、Harness、网关或 Node，所有会话都在被控的完整端（Mac、Windows）上运行。应用使用 Kotlin、Jetpack Compose 与 Material 3，经 remote-only UniFFI／iroh 调用被控端的 `VeluneApplication`，与完整端自己的界面使用同一组具名业务方法。支持 API 28 及以上的 arm64-v8a 设备，应用 ID 为 `app.velune.android`。

## 界面

- **设备**：列出已配对设备及连接状态。单击打开设备；长按可移除本机拨号记录，被控端授权需在那台设备上撤销。
- **添加设备**：扫描完整端“设置 › 设备 › 添加控制设备”显示的二维码（CameraX＋ZXing，不依赖 Google Play 服务），也可以粘贴邀请。读到邀请后先显示设备名；连接后两端显示同一组八位核对码，由被控端允许。Rust 会跟进配对结果，确认后自动进入该设备。
- **设备会话**：标题栏可切换到另一台已配对设备，界面不变，只换目标设备。列表显示运行时、更新时间、项目，以及运行中、等待回应、失败和未读状态。“新会话”需要选择运行时和项目。
- **会话**：用户消息在右，LLM 在左，系统和 Harness 状态居中，不显示头像或昵称。正文可选择复制；思考和工具输出默认折叠，较大的内容在展开时读取。审批与提问显示在输入框上方；输入框支持选择模型（运行时提供时）、发送和停止。

列表和会话只在可见且应用位于前台时轮询，回到后台即停止，被控端继续运行。附件、会话管理、提供商与资源配置不在此 MVP 内。

## 构建与安装

开发机需要 JDK 17、Android SDK 36、Build Tools 35.0.0、NDK `30.0.14904198`，以及 Rust 的 `aarch64-linux-android` target。Gradle wrapper 固定为 8.13，AGP 8.11.1，Kotlin 与 Compose compiler 2.2.21，Compose BOM 2025.10.01，CameraX 1.4.2，ZXing core 3.5.3。

```sh
rustup target add aarch64-linux-android
scripts/build-android.sh
adb install -r app/android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n app.velune.android/.MainActivity
```

`VeluneAndroid.onCreate` 先装载 so 并调用 `NativePlatform.initialize(applicationContext)`，为 iroh 提供 Android DNS 和系统证书验证所需的 JNI context；只靠 JNA 装载不够。生成的 Kotlin 绑定与 so 都是构建产物，不入库。

## 连接与存储

设备身份和远控状态位于 `noBackupFilesDir/remote`。应用禁用系统备份与设备转移，避免在另一台手机上复制出同一身份；卸载或清除数据后需要重新配对。邀请只留在内存中，用过即丢弃。

端点使用 iroh 公共开发 relay 与 discovery，路径策略为 Automatic。`NetworkMonitor` 把默认网络和 LAN 的变化合并成具名快照报告给 Rust；连接恢复由 Rust 的控制器负责，应用只报告当前查看的设备和前后台状态。写操作的结果不明时，Rust 保留未决记录，不会自动重发。
