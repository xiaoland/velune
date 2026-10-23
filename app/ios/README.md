# iPad 与 iPhone 控制端

SwiftUI 通用应用，最低 iOS／iPadOS 17，应用 ID 为 `app.velune.ios`。iPad 使用系统分栏，iPhone 使用同一套视图的紧凑导航。与 Android 一样只控制桌面设备，通过 remote-only UniFFI／iroh 调用 `VeluneApplication`，不携带本地 Agent、Node 或网关。

支持设备邀请扫码与粘贴、核对码配对、设备切换与移除；会话分组、折叠、搜索、刷新与新建；增量消息、Markdown、代码与表格复制、工作过程折叠、按需读取大输出、模型选择、发送、停止、审批与提问。`agents://` 系统链接选择设备后解析，正文链接沿用当前设备；多匹配和部分读取失败显示选择列表。分组、排序与折叠消费被控端核心的列表契约。搜索作用于已加载的会话；组内可以加载更多。

应用在前台且视图可见时轮询，后台不执行 Agent；被控端继续运行。NWPathMonitor 将网络变化报告给 Rust，由共享核心恢复连接。设备身份与配对状态位于 Application Support/remote，排除系统备份，卸载后需重新配对。邀请只保存在内存。连接使用现有公共开发 relay/discovery。

附件发送、会话管理、提供商和资源设置与 Android MVP 一样不在范围内。已有图片显示附件标识；不提供图片预览。相机扫码需要支持 VisionKit 扫描的设备与相机权限，模拟器使用粘贴邀请。

## 构建

需要 Xcode 26.2 或兼容 Swift 6.2 的版本、仓库 Rust 工具链与以下 targets：

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
scripts/build-ios.sh
```

默认构建 Apple Silicon 模拟器，产物为 `target/ios/Build/Products/Debug-iphonesimulator/Velune.app`。设备构建使用：

```sh
scripts/build-ios.sh iphoneos DEVELOPMENT_TEAM=<Apple团队ID> -allowProvisioningUpdates
```

也可以先运行 `scripts/build-ios-native.sh iphoneos`，再打开 [Velune.xcodeproj](Velune.xcodeproj)，选择签名团队与 iPad／iPhone，直接 Run。工程只有一个 app target；生成绑定、静态库和许可证均在 `target`，不入库。Xcode build phase 按当前 SDK 与 Debug／Release 配置构建 Rust。暂只构建 arm64，不维护 Intel 模拟器或 XCFramework。

若本机将 Cargo registry 符号链接到另一磁盘，Askama 0.16 的相对模板路径可能生成失败；用 `CARGO_HOME` 指向 registry 的实际父目录运行构建，不修改第三方源码。本次开发机为 `CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo`。

Swift Markdown 复用 Mac 已采用的 0.9.0，解析后由 SwiftUI 原生组件呈现。Rust 组件许可按实际 iOS target 生成，Swift Markdown／cmark 许可复用 Mac 持有的原文，随包在“关于与开源许可”展示。图标来自共享 graphite SVG 光学稿，采用不透明全幅 iOS 背景。

## 最小验证入口

模拟器安装后以 `--preview` 启动，只展示合成会话，不创建远控端点、不读配对状态或真实会话。Debug 预览也检查增量快照保留／删除消息，以及同一消息的工作过程与正文分割不重叠。

```sh
xcrun simctl install <模拟器ID> target/ios/Build/Products/Debug-iphonesimulator/Velune.app
xcrun simctl launch <模拟器ID> app.velune.ios --preview
```

不带参数为正常设备界面。真机签名、扫码和真实远控闭环需要实际设备验证；构建成功不代表这些流程已验收。
