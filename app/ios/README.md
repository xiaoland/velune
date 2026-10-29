# iPad 与 iPhone 控制端

SwiftUI 通用应用，最低 iOS／iPadOS 18，应用 ID 为 `app.velune.ios`。iPad 使用系统分栏，iPhone 使用同一套视图的紧凑导航。与 Android 一样只控制桌面设备，通过 remote-only UniFFI／iroh 调用 `VeluneApplication`，不携带本地 Agent、Node 或网关。

支持设备邀请扫码与粘贴、核对码配对、设备切换与移除；会话分组、折叠、搜索、刷新与新建；增量消息、Markdown、代码与表格复制、工作过程折叠、按需读取大输出、模型选择、发送、停止、审批与提问。`agents://` 系统链接选择设备后解析，正文链接沿用当前设备；多匹配和部分读取失败显示选择列表。分组、排序与折叠消费被控端核心的列表契约。搜索调用核心 `searchConversations`，覆盖未加载的历史；组内可以加载更多。

每个原生窗口独立保存设备、会话、导航、输入草稿、模型选择与阅读位置，共享一个远控 endpoint、配对身份和 NWPathMonitor；每窗使用稳定的核心 viewID。打开同一会话的两个窗口不会共享输入草稿。草稿存放在本机，切换会话与重启后恢复；未发送会话出现在“草稿”分组，删除只移除该本地草稿。关闭窗口释放核心视图，不关闭其它窗口的连接。

已加载正文在活动窗口显示后，按核心列表水位标为已读；后台列表轮询不改变已读状态。会话长按菜单可以标记已读／未读，也可在新窗口中打开。大纲使用核心用户消息索引并直接跳转；关联 Agent 面板列出父会话、原生子 Agent 与 A2A 协作者，支持打开、返回和新窗口。原生子 Agent 会话只读，不提交独立的列表已读标记。问题面板集中保存操作、设备／会话上下文与诊断详情，可复制和清除，同一持续问题不会被轮询反复添加。

应用在前台且视图可见时轮询，后台不执行 Agent；被控端继续运行。NWPathMonitor 将网络变化报告给 Rust，由共享核心恢复连接。设备身份与配对状态位于 Application Support/remote，排除系统备份，卸载后需重新配对。邀请只保存在内存。连接使用现有公共开发 relay/discovery。

附件发送、远端会话重命名／删除／归档、提供商和资源设置不在范围内。已有图片显示附件标识；不提供图片预览。相机扫码需要支持 VisionKit 扫描的设备与相机权限，模拟器使用粘贴邀请。

聊天交互参考 [Lody](https://github.com/LodyAI/Lody) 与 [HAPI](https://github.com/tiann/hapi)：工作过程默认折叠，工具显示执行状态；用户消息靠右收窄，长按可复制正文；模型选择位于输入框底部。向上阅读后暂停自动跟随，使用“回到最新”恢复；点击发送后收起键盘；⌘Return 发送保留输入焦点，Return 换行，滚动也可收起键盘。滚动跟随使用 iOS 18 原生几何与滚动阶段回调，区分内容增长与用户回看。审批与提问位于输入区上方，可滚动阅读，选项显示标签，提交仍使用核心 ID。仅参考交互，没有复制其业务协议或应用源码。

## 键盘与布局

会话列表在左侧，大纲／关联 Agent 使用原生检查器，窄窗口由系统调整为面板。问题使用独立 sheet，列表只显示操作、上下文与时间，完整诊断在二级页面复制和清除。新建直接进入空草稿并聚焦输入，运行时和项目在输入区就地选择；默认延续当前窗口的有效运行时与工作目录。运行时、项目和模型的当前值始终显示。新草稿的初始模型读取核心 ComposerChoices 的新会话策略，手动选择按草稿和运行时记住。已有会话显示实际运行时和工作目录，仍不提供跨运行时续聊或改执行目录。

会话列表以标题和未读／运行状态为主，时间只显示时分或日期；顶部标题限制为一行，并显示当前设备。检查器只有一个工具栏入口，内部切换大纲和关联 Agent；紧凑窗口跳转后收起面板。普通列表切换不进入返回栈，新建和关联 Agent 跳转保留来源；子 Agent 沿用父会话面板给出的名称。

| 快捷键 | 动作 |
| --- | --- |
| ⌘N / ⇧⌘N | 新会话 / 新窗口 |
| ⌘F / ⌘L | 搜索会话 / 聚焦输入 |
| ⌘Return / ⌘. | 发送 / 停止 |
| ⇧⌘O / ⇧⌘A / ⇧⌘M | 大纲 / 关联 Agent / 问题 |
| ⌘[ / ⇧⌘↓ | 返回上个会话 / 回到最新 |
| Escape | 关闭打开的面板 |

输入器使用 SwiftUI 原生 TextEditor，从一行开始随内容增长，超过六行后在输入区内滚动。运行时与项目选择不会重建草稿，发送在异步操作前固定文字、运行时、目录和模型；创建成功而发送失败时保留已创建身份及文字。窗口存储兼容此前草稿记录。快捷键由 SwiftUI Commands 和当前窗口的 FocusedValues 分发，显示在系统快捷键提示中；输入区处理 Command-Return，并保留普通 Return 的换行与输入法行为。

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

也可以从 Mac 应用的“设置 → 设备 → 安装到 iPad…”选择源码目录、签名团队和设备，构建并保留数据覆盖安装。该入口调用同一构建脚本；`VELUNE_IOS_DEVICE_ID` 选择具体设备，`VELUNE_IOS_DERIVED_DATA` 指定本次构建目录，未设置时沿用默认值。

也可以先运行 `scripts/build-ios-native.sh iphoneos`，再打开 [Velune.xcodeproj](Velune.xcodeproj)，选择签名团队与 iPad／iPhone，直接 Run。工程只有一个 app target；生成绑定、静态库和许可证均在 `target`，不入库。Xcode build phase 按当前 SDK 与 Debug／Release 配置构建 Rust。暂只构建 arm64，不维护 Intel 模拟器或 XCFramework。

若本机将 Cargo registry 符号链接到另一磁盘，Askama 0.16 的相对模板路径可能生成失败；用 `CARGO_HOME` 指向 registry 的实际父目录运行构建，不修改第三方源码。本次开发机为 `CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo`。

Markdown 渲染复用 [MarkdownUI 2.4.1](https://github.com/gonzalezreal/swift-markdown-ui)，表格、列表和行内格式交给库，应用只定制代码块复制与横向滚动。该库已进入维护模式，但支持当前 iOS 18 基线；不为渲染引入另一套视图框架。Rust 组件许可按实际 iOS target 生成，MarkdownUI、NetworkImage 与 cmark 的许可原文随包在“关于与开源许可”展示。图标来自共享 graphite SVG 光学稿，采用不透明全幅 iOS 背景。

## 最小验证入口

模拟器安装后以 `--preview` 启动，复用正式会话行、聊天、输入与面板组件展示合成工作台，不创建远控端点、不读配对状态或真实会话。Debug 预览也检查增量快照保留／删除消息、同一消息的工作过程与正文分割，以及切换运行时后草稿身份、模型记忆和序列化恢复。预览中的新会话、目标选择与问题详情可离线操作，不验证远端发送。追加 `--long` 显示长会话与未闭合代码围栏；“模拟新消息”只更新合成快照，用于检查跟随滚动。

```sh
xcrun simctl install <模拟器ID> target/ios/Build/Products/Debug-iphonesimulator/Velune.app
xcrun simctl launch <模拟器ID> app.velune.ios --preview
```

不带参数为正常设备界面。真机签名、扫码和真实远控闭环需要实际设备验证；构建成功不代表这些流程已验收。


## 本机遥测

遥测设置属于当前客户端，默认未配置且关闭；更换被控设备不改变本机出口。设置页支持 OTLP/gRPC 地址、认证、trace 采样、两侧生效状态和显式测试，以及独立授权的一次性配置复制。保存/复制保留本机开关；分享授权与控制授权分别核对和撤销。手机在前台可提供配置分享，不增加本地执行能力或后台保活，也不保证后台仍可读取。

使用官方原生 SDK 与 Rust SDK，共用实例身份与配置修订。原始会话、路径、认证和底层错误链不外传，本地问题详情保留。接收回执不等于后台查询已可见，未提供的 SDK 丢弃计数为未知。开发启动与隔离验收见[开发遥测](../../docs/development.md#开发遥测)，跨单元契约见[遥测装配](../../docs/design/telemetry.md)。
