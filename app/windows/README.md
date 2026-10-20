# Windows 完整端

Windows 与 Mac 一样是完整端：在本机运行 Agent、Harness 与网关，也能控制其他已配对的完整端。界面使用 [GPUI](https://www.gpui.rs/)（`gpui-kit` 固定为 `=0.7.3`）自绘，这是原生视觉原则下用户接受的例外，交互仍尽量遵循 Windows 习惯。应用与核心同为 Rust，直接链接 `velune-bindings` 的 Rust API（默认 feature 为 `local-runtime` 与 `remote`），不经 UniFFI 生成的外语绑定，也不启动常驻 Host 进程。

## 界面

- **主窗口**：左侧是会话列表，顶部是设备切换，可在“本机”、已配对设备与“添加设备…”之间切换。切换后界面不变，只换目标设备，本机与远端都通过同一个 `VeluneApplication` 访问。列表隐藏已归档会话与子 Agent，显示运行中、等待回应和失败状态。单击打开会话，双击就地编辑标题：Enter 保存，点击别处或按 Esc 取消。
- **新会话**：在会话区顶部选择运行时，并选择或输入项目目录（本机可浏览），然后直接发送第一条消息，会话在发送时创建。
- **会话**：用户消息在右，LLM 在左，系统和 Harness 状态居中，不显示头像或昵称。正文按 Markdown 渲染，可选择复制；思考和工具输出默认折叠，较大的内容在展开时读取。审批与提问显示在输入框上方。输入框可选择模型（运行时提供时），Ctrl+Enter 发送，运行中可停止。
- **设置**（单例窗口）：
  - **设备**：设备名与接入开关；生成邀请，显示二维码、邀请文本并可复制；列出入站授权并可撤销；粘贴邀请以控制另一台设备；列出联系人及配对状态。有设备请求控制本机时，主窗口弹出批准对话框，显示请求方名称和分组核对码，Enter 和 Esc 不触发任一按钮。
  - **运行时**：列出运行时实例，可启用、停用或删除。可从 PATH 与 pnpm、npm、Codex 的全局目录中发现安装，探测版本后添加；也可手动填写。
  - **提供商**：列出默认网关的提供商，可删除；添加表单从 core 目录选择 HTTP＋API key 模板，包含 Kimi Coding Plan。普通 API 模板可编辑协议与地址；订阅模板保留固定组合，填写 key 和模型 ID，不需要 OAuth 登录。

附件、会话管理（归档、删除）、资源管理、OAuth 订阅登录与用量页不在此 MVP 内；这些配置可在 Mac 上完成。跨端配置同步尚在方案阶段。

## 构建与运行

需要 Rust（MSVC 目标，版本同 `rust-toolchain.toml`）、Visual Studio Build Tools、Go 与 llvm-mingw，见 [开发说明](../../docs/development.md#windows-构建)。runtime-support 先按开发说明安装到 `target/runtime-support`，再执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
target\windows\Velune\velune.exe
```

脚本输出 `target\windows\Velune`，其中包括 `velune.exe`、它加载的 `velune_relaykit.dll`，以及 Harness 辅助脚本、`node_modules` 和第三方许可组成的 `resources`。整个目录可以移动，但三者必须放在一起。release 构建不弹控制台窗口；debug 构建保留控制台，便于看日志。

## 数据与 Harness

应用数据位于 `VELUNE_HOME`，缺省为 `%USERPROFILE%\.velune`。远控身份与状态在其中的 `remote` 目录。设备名取自 `COMPUTERNAME`。

Harness 原生运行于 Windows，不经 WSL 或 SSH runtime。npm／pnpm 为每个命令生成 `.cmd`、`.ps1` 和无扩展名的 `sh` 脚本三种启动器。Pi 的安装解析只读取 `sh` 脚本里的目标路径，所以发现时使用这一个。Codex 与 Claude Code 直接启动二进制，优先 `.exe`，否则用 `.cmd`。Pi 需要 Node 的绝对路径，发现时取 PATH 上第一个 `node.exe`。子进程放在 kill-on-close 的 Job Object 中，应用退出（包括被强制结束）时，Harness 及其子进程随之结束。

首次冷启动时，Windows Defender 扫描可能使 Pi 的版本探测超过 15 秒而失败，重试即可。

## 已知限制

- gpui-component 0.7.3 在 Windows 上，对话框内的输入框收不到点击：点击落到遮罩上，对话框随之关闭。因此新会话与重命名改为就地编辑，对话框只用于按钮确认。
- SSH 运行时依赖 OpenSSH ControlMaster 与 Unix socket，Windows 宿主不支持。
- 中文输入法组字中途切换到另一个输入框时，未上屏的拼音会留在原输入框，组字继续在新输入框进行。GPUI 所有输入框共用一个原生窗口，输入法感知不到焦点变化。

ARK Coding Plan 模板的添加表单另有 AK/SK 与本次关联确认，和模型 API key 一起原子保存；无查询认证可先使用无上限模型调用。该确认由用户负责，未自动核验账户，仍无独立额度页或提供商编辑页。
