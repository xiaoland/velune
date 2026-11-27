# Android 控制端

本文面向 Android 端开发者，记录平台实现、构建与验证。安装和配对操作见[用户指南](../../docs/usage.md)，代码 Agent 的 ADB 操作约束见 [AGENTS.md](AGENTS.md)。

Android 是只控制、不执行的端。它不运行 Agent、Harness、网关或 Node，所有会话都在被控的完整端（Mac、Windows）上运行。应用使用 Kotlin、Jetpack Compose 与 Material 3，经 remote-only UniFFI／iroh 调用被控端的 `VeluneApplication`，与完整端自己的界面使用同一组具名业务方法。支持 API 28 及以上的 arm64-v8a 设备，应用 ID 为 `app.velune.android`。

## 界面

布局与交互参考同类开源移动客户端 HAPI、Lody 与 Happy（只借鉴可见的布局与交互，代码独立编写，见 [NOTICE](licenses/NOTICE.md)），用 Material 3 呈现：Android 12 起跟随系统动态色，更早的系统用由品牌色生成的配色。

- **会话列表（首页）**聚合所有已配对设备的会话。多设备时可按设备筛选，也可按标题、项目或设备搜索。列表按“等待回应”“置顶”“最近”分段，行内显示运行时缩写、标题、相对更新时间，以及运行中、等待回应、失败和未读状态。列表内会标明不可达的设备，其余设备的会话仍可浏览。下拉刷新全部可见设备，右上角菜单可打开“设备”“外观与显示”“遥测”和“关于”。没有已配对设备时显示“添加设备”。
- 点击“**+ 新会话**”直接进入输入页，可在输入框上方切换设备、运行时和项目。项目面板按名称或路径搜索最近项目，也可使用运行时默认目录。同一设备内切换运行时或项目时保留草稿，切换设备时带上正文，已上传的附件留在原设备。
- **外观与显示**可分别调整界面、消息和代码字号，以及显示密度。设置页的样例随调整实时更新，选项自动保存到本机，也可恢复默认值。
- **设备**：从会话列表菜单进入，以标准 Material 3 列表展示已配对设备与连接状态。点击确认后的设备可返回并筛选该设备的会话；“更多”菜单可移除本机拨号记录，被控端授权需在那台设备上撤销；右上角“连接设置”管理本机中继。
- **添加设备**：顶部可选择“扫码”或“邀请链接”。扫码读取完整端“设置 › 设备 › 添加控制设备”显示的二维码（CameraX＋ZXing，不依赖 Google Play 服务）；邀请链接可编辑、粘贴，也支持点击 `velune://pair/v3/...` 进入同一预览，无需相机权限。所有入口使用共享核心验证，先显示设备名，由用户明确点击“连接”；随后两端显示同一组八位核对码，由被控端允许。旧 JSON 邀请仍可粘贴导入。Rust 会跟进配对结果，确认后自动返回会话列表。
- **xURL**：点击 `agents://` 深链自动查询并打开会话。正文链接沿用当前会话设备；系统链接在只有一台已配对设备时直接使用它，多台时先选设备。唯一匹配且没有读取失败时直接打开，多匹配或部分失败时供选择；没有设备时沿用现有添加设备流程。无需输入地址或点击查找，Android 不分发或执行桌面 xurl CLI。
- **会话正文**上方显示标题与运行时、项目、设备，连接异常时另有状态提示。顶部可手动刷新正文，有至少两轮用户输入时可用“大纲”跳转。用户消息在右侧气泡显示，LLM 回复全宽显示 Markdown，表格和代码块可横向滚动并复制。用户消息与已完成回复显示本地时间戳，支持一键复制。共享核心按会话状态开放编辑或分叉操作，编辑后可选择在原会话或新会话中继续。未结束、失败或取消的轮次默认沿时间线展开工作过程，正常完成后折叠为一行，显示工具数和用时。带摘要的思考块默认折叠，较大的工具输出在展开时读取。离开底部时可点击“回到最新”。
- **消息输入框（Composer）**在软键盘弹出时贴合键盘上沿，收起后恢复底部圆角和导航栏留白。审批与提问显示在输入框上方。可用模型、推理强度和权限模式由共享 `NextTurn` 给出，可沿用当前值或使用运行时默认值。已有会话允许切换且被控设备有多个运行时时，可在“执行配置”切换下一轮的运行时，随之加载对应的模型与执行选项。附件通过系统文件选择器多选，单个文件上限为 20 MiB。文件上传到当前被控设备并加入草稿，可在发送前移除。发送后的消息在被控端确认前显示“发送中”。
- **执行控制**由共享核心按会话状态开放。运行中可用“排队”（Queue）把消息加入发送队列，或用“介入”（Steer）将消息交给当前轮次。介入沿用当前运行时和模型。点开输入框上方的队列摘要可查看排队项。失败后若可恢复，且当前选择仍为会话运行时，可使用“继续执行”（Resume）。更多菜单按状态提供“压缩上下文”和“释放会话控制权”。
- **后台任务**和 **Agent 协作**面板都可从已有会话的更多菜单打开。后台任务面板可刷新列表，展开条目时按需读取可用输出，也可停止允许停止的运行中任务。Agent 协作面板展示父会话、子 Agent 及 A2A 协作会话，可打开关联会话，并在共享核心开放控制时停止子 Agent。
- **关于**：版本、界面参考声明与随应用分发组件的许可全文。

列表和会话只在可见且应用位于前台时轮询，回到后台即停止，被控端继续运行。打开或返回会话列表时立即读取，成功后每隔 30 秒刷新，读取失败后间隔 5 秒重试。首屏读到后，活动标记单独更新，有活跃会话时每 2 秒读取一次，空闲时每 5 秒读取一次。会话正文独立显示和刷新，输入区需完成同步才能发送。

各屏的 `…Content` 通过参数接收业务状态，通过回调提交操作。`TranscriptEntries`、`SessionSections` 等逻辑可独立于 Compose 使用。

## Debug 场景目录

Debug APK 内置 `GalleryActivity`，复用应用界面并用虚构数据响应操作，可在真机上验证交互，无需配对设备。把 `YOUR_DEVICE_SERIAL` 换成 `adb devices -l` 中的设备序列号。第一条启动命令打开目录，第二条用 `screen` 参数直达场景。打开场景后可用系统返回键回到目录。

```sh
android_serial=YOUR_DEVICE_SERIAL
adb -s "$android_serial" shell am start -S -W -n app.velune.android/.GalleryActivity
adb -s "$android_serial" shell am start -S -W -n app.velune.android/.GalleryActivity --es screen conversation
```

| 场景 ID | 可以查看或操作 |
| --- | --- |
| `gallery` | Material 3 场景目录，省略 `screen` 时默认打开。 |
| `sessions` | 多设备聚合会话列表，可筛选、搜索、打开会话或直达新会话。兼容旧 ID `list`。 |
| `empty-sessions` | 已有配对设备、暂无会话的列表。 |
| `no-devices` | 暂无配对设备的首页。 |
| `devices` | 已配对设备及连接状态。 |
| `conversation` | 已完成会话，可选运行时与执行配置、操作附件、编辑消息、分叉、手动刷新、查看任务输出和关联会话。 |
| `running` | 展开的工作过程、审批、可停止的后台任务和子 Agent、排队与介入，以及大纲跳转。 |
| `failed` | 失败轮次及“继续执行”操作。 |
| `new` | 直接打开新会话，可切换设备、运行时或搜索项目，切换时保留草稿正文。 |
| `appearance` | 实时预览界面、消息、代码字号与显示密度，可保存或恢复 Gallery 专用设置。 |

`app/src/debug/kotlin/app/velune/android/preview/` 中的 [PreviewFixtures.kt](app/src/debug/kotlin/app/velune/android/preview/PreviewFixtures.kt) 按领域组织设备、运行时与模型目录、权限模式、会话、后台任务与输出、Agent 协作、附件及排队项等样本。[PreviewCatalog.kt](app/src/debug/kotlin/app/velune/android/preview/PreviewCatalog.kt) 集中登记场景的 ID、中文标题、说明和分组。扩展样本时在 `PreviewFixtures.kt` 增加数据函数，新增可进入的场景时在 `PreviewCatalog.kt` 注册。

会话操作使用内存中的模拟状态，重新启动 Gallery 后从初始样本开始。外观调整实时应用到 Gallery，点击“保存预览设置”后，下次启动 Gallery 会沿用保存值。点击“恢复已保存”可撤回尚未保存的调整。

## 构建与安装

开发机需要 JDK 17、Android SDK 36、Build Tools 35.0.0、NDK `30.0.14904198`，以及 Rust 的 `aarch64-linux-android` target。Gradle wrapper 固定为 8.13，AGP 8.11.1，Kotlin 与 Compose compiler 2.2.21，Compose BOM 2025.10.01，CameraX 1.4.2，ZXing core 3.5.3，commonmark-java 0.24.0。

```sh
rustup target add aarch64-linux-android
svc run build-android
adb install -r app/android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n app.velune.android/.MainActivity
```

开发入口默认使用 Rust dev（有限调试信息与增量缓存）和 debug APK；`scripts/build-android.sh --profile release` 显式构建 Rust release 与未签名 release APK，发布签名仍由分发流程负责。`--diagnostics` 可选择诊断特性。SVC 合并同工作树的在途请求；移动端共用 `target/cross-rust` 中按 triple 区分的 Rust 缓存，以及 `target/build-tools` 中的一份 host 生成器。Kotlin 绑定和 so 位于稳定的 `target/android-bindings`、`target/android-native`，内容未变时不更新文件，保持 Gradle 增量输入稳定。

只修改 Kotlin 时可用 `scripts/build-android.sh --skip-native`；此入口先检查所选 profile 的 native 库与已生成绑定的摘要和配置是否配套。缺少或不匹配时失败，需先执行正常构建。跳过编译不证明 native 库已包含此后修改的 Rust 源码。开发构建不安装或启动应用。

`scripts/build-android-native.sh` 同时运行 `scripts/android-licenses.py`，按实际链接的 Rust crate 生成 `target/android-licenses/rust-licenses.txt`，并将仓库根目录的 Velune `LICENSE` 一起作为 assets 打包；它们与 [licenses](licenses) 下的 NOTICE 和第三方许可全文一起在“关于”中显示。“关于”还按应用版本链接到对应 GitHub release 源码归档。新增 Java／Kotlin 依赖时同步更新 NOTICE。

`VeluneAndroid.onCreate` 先装载 so 并调用 `NativePlatform.initialize(applicationContext)`，为 iroh 提供 Android DNS 和系统证书验证所需的 JNI context；只靠 JNA 装载不够。生成的 Kotlin 绑定与 so 都是构建产物，不入库。

## 连接与存储

设备身份和远控状态位于 `noBackupFilesDir/remote`。应用禁用系统备份与设备转移，避免在另一台手机上复制出同一身份；卸载或清除数据后需要重新配对。邀请只留在内存中，用过即丢弃。配对链接的冷启动与暖 Intent 都新压入添加页，保留此前页面及进程内草稿；应用消费链接后清除 Activity 的数据 URI，不把邀请放入 Bundle、SavedState、偏好或应用日志。

设备页的“连接设置”管理本机中继：可选公共预设、自定义地址列表或关闭中继，并按服务地址添加、替换或移除 Bearer 访问凭据。已有令牌不读回表单，留空保持；更换中继地址不会携带旧地址的凭据。保存由共享 Rust 核心持久化并应用网络换代，重开后沿用，设备身份与配对保持。设置应用状态与设施实际可达状态分别显示；应用失败可重试，修订冲突或字段错误保留编辑。地址查找保持现有设置，默认使用公共 discovery，路径策略为 Automatic。`NetworkMonitor` 把默认网络和 LAN 的变化合并成具名快照报告给 Rust；连接恢复由 Rust 的控制器负责，应用只报告当前查看的设备和前后台状态。写操作的结果不明时，Rust 保留未决记录，不会自动重发。

Debug 应用还提供[本地配对 CLI](../../docs/development.md#本地配对-cli)，通过 ADB 的 `run-as` 调用当前应用服务；不需要操作界面或重开远控实例。


## 本机遥测

遥测设置属于当前客户端，默认未配置且关闭；更换被控设备不改变本机出口。设置页支持 OTLP/gRPC 地址、认证、trace 采样、两侧生效状态和显式测试，以及独立授权的一次性配置复制。保存/复制保留本机开关；分享授权与控制授权分别核对和撤销。手机在前台可提供配置分享，不增加本地执行能力或后台保活，也不保证后台仍可读取。

使用官方原生 SDK 与 Rust SDK，共用实例身份与配置修订。原始会话、路径、认证和底层错误链不外传，本地问题详情保留。接收回执不等于后台查询已可见，未提供的 SDK 丢弃计数为未知。开发启动与隔离验收见[开发遥测](../../docs/development.md#开发遥测)，跨单元契约见[遥测装配](../../docs/product-tdd/telemetry.md)。

## 执行目标准备

会话模型在内容同步之后调用共享 `prepareComposerTarget`，采用保存端返回的实际目标与修订，随后用共享解析结果处理目录到达和有效发送参数。首次打开的默认值继续继承，明确选择只保存对应字段。内容身份独立于目标准备，失败时保留正文、本地未同步选择和 CAS 基线，迟到回复按目标身份拒收。

运行时目录随所选目标加载。如果目标运行时失效或属于另一执行端，应用保留已恢复的正文和编辑能力，待选定有效目标后才恢复发送。
