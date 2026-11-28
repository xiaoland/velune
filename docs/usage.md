# 安装与开始使用

本页面向 Velune 使用者，说明安装、添加运行时和连接桌面设备。参与开发请从[开发说明](development.md)开始；产品需求和技术设计分别见[产品规范](prd/index.md)与[技术契约](product-tdd/index.md)。

## 平台与安装

从 GitHub Releases 下载对应平台的预览包，并按该版本的发行说明安装。公开预览版提供 macOS 15+（Apple Silicon／Intel）和 Android 9+ arm64 构建。Windows 与 iPhone 目前使用源码构建。iPad 可通过 Mac 应用使用自己的 Apple 签名团队安装。 [Releases](https://github.com/xiaoland/velune/releases)

macOS 包采用 ad-hoc 签名且尚未公证。首次被系统拦截时，可在“系统设置 → 隐私与安全性 → 仍要打开”针对 Velune 放行，保持 Gatekeeper 开启。

Android 使用已发布的签名 APK。已有安装能否覆盖升级取决于应用身份与签名；遇到签名冲突时保留旧安装及数据，不通过卸载解决。卸载会删除应用私有数据和设备配对。

没有适用发布资产，或希望自行构建时，按[平台构建说明](development/platforms.md)准备工具链；开发用的 SVC 和合成数据不是普通使用的前提。

## 添加运行时

Velune 不附带 Codex、Claude Code、Pi 等 Harness。先安装要使用的 Harness，再在 Velune 中添加运行时，确认其可执行文件位置。账户与 API 凭据由你配置。

打开已有会话或新建会话，选择 Harness 的原生模型连接，或在 Velune 中配置自己的 AI 提供商、模型与路由。桌面端执行工具并持有原生会话历史；Velune 提供会话界面与协调能力。

适配器兼容版本、SSH 执行位置与 HOME 规则见[运行时技术说明](../packages/agent-runtime/README.md#版本与原生控制)。遇到问题时，一并记录 Velune、操作系统与 Harness 的版本。

### Antigravity

1. 在运行 Agent 的电脑上安装 Antigravity CLI 1.5.1 或更新版本，并在终端运行 `agy acp --login` 完成登录。已经登录 Antigravity CLI 时可以直接继续。 [Google ACP 指南](https://antigravity.google/docs/cli/acp/)
2. 在 Velune 的“设置 → Agent 运行时 → 快速导入”中添加 Antigravity。手动添加或配置 SSH 执行时，选择“ACP v1”，填入执行机器上 `agy` 的绝对路径，并添加单独的启动参数 `acp`。
3. 新建会话时选择 Antigravity。打开会话后，可使用 Antigravity 公布的模型和会话选项。

### 在其他运行时使用 Antigravity 订阅

在对应的系统用户或 SSH 主机上用官方 `agy` 完成登录，再在 Velune 的「设置 → AI 提供商 → 添加」选择 **Google Antigravity 订阅**。填写该环境上的 `agy` 可执行文件绝对路径和 ACP 提供的模型 ID。macOS 使用 SSH 时填写已有的 OpenSSH 主机别名或 `user@host`。远端需要 Python 3 和可用的 SSH 回环端口转发。Windows 使用本机 `agy`。将分别登录的提供商模型加入同一条模型路由，即可从支持网关的运行时选择。Google 会按登录和客户端提供可用模型。 [Google ACP 模型与登录说明](https://antigravity.google/docs/ide/extensions/acp/#models-and-thinking-level)

额度查询需要 `agy` 1.5.0 或更新的稳定版本。保存提供商后，详情会自动读取额度，也可点击「刷新」。可查看模型组余额、重置时间和执行目标的历史。已匹配模型的额度窗口可设置使用上限，也供 Smart 路由和额度提醒使用。如果在执行环境中切换了 agy 登录，可以手动刷新并清除该目标原有的额度历史。 [Google 额度说明](https://antigravity.google/docs/cli/commands/usage/)

## 使用 Google Gemini API

在运行 Agent 的电脑打开「设置 → AI 提供商」，添加 **Google Gemini API** 模板，填入 Google AI Studio 创建的 Gemini API key 和要使用的模型 ID。保存后可在支持网关的运行时中选择该模型，也可将它加入模型路由。 [Google API key 说明](https://aistudio.google.com/docs/api-key)

## 在 Mac 查找会话

侧栏搜索也能找到列表尚未加载的会话。在侧栏的「显示」菜单打开「已归档会话」，可按标题搜索、筛选、取消归档或删除会话。读取失败时，可在错误旁重试。

## 手机与 iPad 连接桌面

1. 保持作为执行端的 Mac 或 Windows 上的 Velune 运行。
2. 在桌面端“设置 → 设备”中生成添加控制设备的邀请。
3. 在手机或 iPad 添加设备，扫描二维码或粘贴邀请。Android 可从首页空态或右上角「更多 → 设备」添加，也可直接点开收到的配对邀请链接进行预览，再点击「连接」。
4. 核对两端显示的核对码，由桌面端批准；批准后各已配对设备的会话会集中显示在会话列表中。

手机和 iPad 本机不运行 Coding Agent。关闭移动端不停止桌面上已经运行的任务；移动端在后台的连接能力受系统限制。

Android 在首页聚合已配对设备的会话，可按设备筛选。点「新会话」直接打开输入页，在输入框上方选择设备、运行时和项目。界面字号、消息字号、代码字号与显示密度可从「更多 → 外观与显示」调整。

### 连接设置

使用默认公共中继即可开始配对。需要自建中继时，在设备页打开“连接设置”，选择自定义中继并填写服务地址；访问令牌与邀请分别管理，不把令牌拼进地址。配对仍需单独批准。

保存后分别查看设置是否已应用以及中继是否可达；保存成功不表示被控电脑已经连上。已保存令牌不回显，按界面的保留、替换或移除操作管理认证。更完整的权限与连接行为定义见[远控产品规范](prd/remote-access.md)。

## 从 Mac 应用安装到 iPad

这一入口使用你自己的 Apple 签名团队构建和安装，需要先准备工具链与对应版本的源码。

1. 安装兼容 Swift 6.2 的 Xcode 和 Rust 工具链，添加 `aarch64-apple-ios` target。在 Xcode 登录自己的 Apple 账号，准备可用于该设备的签名团队。工具链命令见 [iOS 构建说明](../app/ios/README.md#构建)。
2. 下载并解压对应发布版本的 Velune 源码，或检出对应 tag。
3. 连接并解锁 iPad，确认信任此 Mac，在 iPad 开启开发者模式。
4. 在 Mac 应用打开“设置 → 设备 → 安装到 iPad…”，选择源码目录，填写签名团队 ID，刷新并选择 iPad，点击“安装”。

后续沿用同一签名团队和应用身份覆盖安装，保留应用数据。源码目录与团队选择会记住，安装版本由所选源码决定。

## 分享到 HAPI

在 Mac 打开本机既有会话，选择“会话 → 分享到 HAPI…”，也可从会话列表或标签菜单打开。确认会话和连接方式后点击“开启共享”。选择 HAPI 公共中继时，Mac 自动建立手机可达的 HTTPS 入口。使用已有 HTTPS 入口时，将整个 origin 转发到 `127.0.0.1:33430`，保持 SSE 流式传输，再在共享页输入该地址。

在 HAPI 客户端配对页扫描 Mac 显示的二维码，或手动填写 HTTPS 地址和令牌。HAPI 可读取选定会话、发送消息、停止当前轮次和处理支持的审批。

关闭二维码页后共享继续，可在“设置 → 设备 → HAPI 会话共享”再次查看二维码或结束共享；退出 Velune 会结束本次共享。

## 反馈问题

报告应用版本、平台、运行时版本、触发步骤与界面显示的错误。分享诊断前移除凭据与私人会话内容，不公开原始会话或未经脱敏的日志。普通问题通过 [Issues](https://github.com/xiaoland/velune/issues) 反馈，安全漏洞使用[私下报告](https://github.com/xiaoland/velune/security/advisories/new)。
