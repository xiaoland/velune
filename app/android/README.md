# Android 控制端

Android 是只控制、不执行的端。它不运行 Agent、Harness、网关或 Node，所有会话都在被控的完整端（Mac、Windows）上运行。应用使用 Kotlin、Jetpack Compose 与 Material 3，经 remote-only UniFFI／iroh 调用被控端的 `VeluneApplication`，与完整端自己的界面使用同一组具名业务方法。支持 API 28 及以上的 arm64-v8a 设备，应用 ID 为 `app.velune.android`。

## 界面

布局与交互参考同类开源移动客户端 HAPI、Lody 与 Happy（只借鉴可见的布局与交互，代码独立编写，见 [NOTICE](licenses/NOTICE.md)），用 Material 3 呈现：Android 12 起跟随系统动态色，更早的系统用由品牌色生成的配色。

- **设备**：已配对设备以卡片列出，状态点表示已连接、连接中、未连接或出错。单击打开设备；“更多”菜单可移除本机拨号记录，被控端授权需在那台设备上撤销。没有设备时显示说明与“添加设备”。
- **添加设备**：顶部可选择“扫码”或“手动输入”。扫码读取完整端“设置 › 设备 › 添加控制设备”显示的二维码（CameraX＋ZXing，不依赖 Google Play 服务）；手动输入支持编辑、粘贴完整端复制的邀请，无需相机权限。两种入口使用同一套核心邀请验证，读到邀请后先显示设备名；连接后两端显示同一组八位核对码，由被控端允许。Rust 会跟进配对结果，确认后自动进入该设备。
- **设备会话**：标题栏显示设备与连接状态，点按可切换到另一台已配对设备，界面不变，只换目标设备。会话按项目分组（置顶在前，分组可收起），行内为运行时缩写标识、标题、更新时间，以及运行中、等待回应、失败和未读状态；支持按标题或项目搜索、下拉刷新。“新会话”在底部面板中选择运行时和项目。
- **xURL**：点击 `agents://` 深链自动查询并打开会话。正文链接沿用当前会话设备；系统链接在只有一台已配对设备时直接使用它，多台时先选设备。唯一匹配且没有读取失败时直接打开，多匹配或部分失败时供选择；没有设备时沿用现有添加设备流程。无需输入地址或点击查找，Android 不分发或执行桌面 xurl CLI。
- **会话**：用户消息在右侧气泡，LLM 回复全宽显示 Markdown（表格、代码块可横向滚动并复制），系统和 Harness 状态居中，不显示头像或昵称。每轮的思考与工具调用折叠为一行“工作过程 · N 个工具 · 用时”，运行时显示最新一步；展开后逐个显示工具卡片，较大的输出展开时才读取。离开底部时出现“回到最新”按钮。审批与提问显示在输入框上方；输入框为圆角容器，内含模型选择（运行时提供时）与发送／停止按钮，刚发送的消息在被控端确认前以“发送中”显示。
- **关于**：版本、界面参考声明与随应用分发组件的许可全文。

列表和会话只在可见且应用位于前台时轮询，回到后台即停止，被控端继续运行。附件、会话管理、提供商与资源配置不在此 MVP 内。

界面层把状态与内容分开：各屏的 `…Content` 只接收数据与回调，`TranscriptEntries`、`ConversationSections` 等不依赖 Compose。debug 包另含 `GalleryActivity`，用虚构数据显示各屏，便于不配对设备也能在真机上检查布局：

```sh
adb shell am start -n app.velune.android/.GalleryActivity --es screen conversation
```

可选 `devices`、`no-devices`、`list`、`empty-list`、`conversation`、`running`、`new`。

## 构建与安装

开发机需要 JDK 17、Android SDK 36、Build Tools 35.0.0、NDK `30.0.14904198`，以及 Rust 的 `aarch64-linux-android` target。Gradle wrapper 固定为 8.13，AGP 8.11.1，Kotlin 与 Compose compiler 2.2.21，Compose BOM 2025.10.01，CameraX 1.4.2，ZXing core 3.5.3，commonmark-java 0.24.0。

```sh
rustup target add aarch64-linux-android
scripts/build-android.sh
adb install -r app/android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n app.velune.android/.MainActivity
```

`scripts/build-android-native.sh` 同时运行 `scripts/android-licenses.py`，按实际链接的 Rust crate 生成 `target/android-licenses/rust-licenses.txt`；它与 [licenses](licenses) 下的 NOTICE 和许可全文一起作为 assets 打包，在“关于”中显示。新增 Java／Kotlin 依赖时同步更新 NOTICE。

`VeluneAndroid.onCreate` 先装载 so 并调用 `NativePlatform.initialize(applicationContext)`，为 iroh 提供 Android DNS 和系统证书验证所需的 JNI context；只靠 JNA 装载不够。生成的 Kotlin 绑定与 so 都是构建产物，不入库。

## 连接与存储

设备身份和远控状态位于 `noBackupFilesDir/remote`。应用禁用系统备份与设备转移，避免在另一台手机上复制出同一身份；卸载或清除数据后需要重新配对。邀请只留在内存中，用过即丢弃。

端点使用 iroh 公共开发 relay 与 discovery，路径策略为 Automatic。`NetworkMonitor` 把默认网络和 LAN 的变化合并成具名快照报告给 Rust；连接恢复由 Rust 的控制器负责，应用只报告当前查看的设备和前后台状态。写操作的结果不明时，Rust 保留未决记录，不会自动重发。

Debug 应用还提供[本地配对 CLI](../../docs/development.md#本地配对-cli)，通过 ADB 的 `run-as` 调用当前应用服务；不需要操作界面或重开远控实例。


## 本机遥测

遥测设置属于当前客户端，默认未配置且关闭；更换被控设备不改变本机出口。设置页支持 OTLP/gRPC 地址、认证、trace 采样、两侧生效状态和显式测试，以及独立授权的一次性配置复制。保存/复制保留本机开关；分享授权与控制授权分别核对和撤销。手机在前台可提供配置分享，不增加本地执行能力或后台保活，也不保证后台仍可读取。

使用官方原生 SDK 与 Rust SDK，共用实例身份与配置修订。原始会话、路径、认证和底层错误链不外传，本地问题详情保留。接收回执不等于后台查询已可见，未提供的 SDK 丢弃计数为未知。开发启动与隔离验收见[开发遥测](../../docs/development.md#开发遥测)，跨单元契约见[遥测装配](../../docs/design/telemetry.md)。
