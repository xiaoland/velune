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

Mac 的远程控制入口创建邀请后，用户在 Android 显式粘贴并配对。应用只在按下“粘贴邀请”时读取剪贴板。邀请和消息正文留在内存，不进入日志、持久配置或备份；成功配对后清空邀请框。身份／可信设备文件位于应用的 `noBackupFilesDir`，应用同时禁用系统备份与设备转移。卸载或清除应用数据会丢失身份，需要重新配对。

当前端点显式选择 iroh 的公共开发 relay／discovery，正常采用 Automatic 选路；Direct 或 Relay 来自实际连接状态。开发设施没有 Velune 生产服务承诺。同 Wi-Fi 真机验收不能证明跨公网 NAT、移动网络切换或长期后台稳定性。

UI 状态由 ViewModel 管理，FFI／网络调用运行于 IO 调度器。进入后台仅暂停观察；执行设备继续工作。回到前台和系统网络变化时调用 `network_change`，随后同步现有连接状态。没有前台常驻 service。应用重开后用户从已配对设备列表重连；“移除此记录”只移除本机拨号记录，执行端授权需要在执行设备撤销。

写操作先把请求编号、所属设备、操作类型与目标保存到私有文件，再提交封闭的具名命令。同一时间只跟踪一个未决写操作；连接丢失后“查询结果”查询原编号，不重新发消息。未知、回执过期、执行端重启都保留待确认状态。用户检查执行结果后可明确“结束查询”；这不会取消执行端的工作。持久文件不保存命令正文。控制连接释放和断开均不调用停止执行；停止必须使用会话里的“停止”操作。
