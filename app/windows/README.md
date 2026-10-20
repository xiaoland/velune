# Windows 完整端

Windows 的完整端：在本机运行 Agent、Harness 与网关，也能控制其他已配对的完整端。界面使用 [MyGO](https://github.com/egoist/mygo)（固定 `v0.5.5`）以纯 Go 自绘，这是原生视觉原则下用户接受的例外，交互仍尽量遵循 Windows 习惯。Go 经 `velune-bindings` 生成的 Go 绑定调用 Rust 核心：运行时用 purego 加载同目录的 `velune_bindings.dll`，不用 cgo，也不启动常驻 Host 进程（见 [bindings](../../packages/bindings/README.md)）。

此前的 GPUI 版已按用户 2026-10-20 的决定删除；进度与验证记录见 [任务](../../tasks/windows-mygo/packet.md)。

## 界面

- **主窗口**：左侧是会话列表，顶部是设备切换，可在“本机”与已配对设备之间切换。切换后界面不变，只换目标设备，本机与远端都通过同一个 `VeluneApplication` 访问。列表隐藏已归档会话与子 Agent，显示运行中、等待回应、失败和未读状态。单击打开会话；双击或 F2 就地编辑标题，Enter 保存，Esc 或焦点离开取消。
- **新会话**：在会话区顶部选择运行时，并选择或输入项目目录（本机可浏览），然后直接发送第一条消息，会话在发送时创建。
- **会话**：用户消息在右，LLM 在左，系统和 Harness 状态居中，不显示头像或昵称。LLM 正文按 Markdown 渲染，回复中途以流式呈现；思考和工具输出默认折叠，较大的内容在展开时读取。审批与提问显示在输入框上方。输入框可选择模型（运行时提供时），Ctrl+Enter 发送，运行中可停止。
- **控制请求**：有设备请求控制本机时，主窗口弹出对话框，显示请求方名称和分组核对码。焦点落在核对码上，Enter 和 Esc 不触发任一按钮，必须点击“允许”或“拒绝”。
- **设置**：侧栏的“设置”按钮或设备切换菜单中的“添加设备…”打开设置窗口，只有一个，再次打开时回到已开的窗口。
  - **设备**：设备名称与“允许其他设备控制这台电脑”开关；创建五分钟内有效、只能使用一次的邀请（二维码与可复制文本），对方使用后或过期即收回；列出可以控制这台电脑的设备（可撤销）；粘贴另一台设备的邀请发起控制，并列出这台电脑可以控制的设备及其连接状态或核对码（可取消或移除）。
  - **运行时**：本机已配置的运行时（启用开关、删除）；在 PATH 与 npm／pnpm、Codex 安装目录中发现 Pi、Codex、Claude Code 并导入，只读取公开版本；或按运行时类型手动填写（路径字段可浏览）。ACP 类型的启动参数与环境不在这里编辑。
  - **提供商**：本机网关的提供商列表（删除）；按 HTTP API key 模板或自定义 API 添加，填写名称、协议、接口地址、API key 与以逗号分隔的模型 ID。API key 保存在应用私有配置中。

## 构建与运行

需要 Rust（MSVC 目标）、Visual Studio Build Tools、Go 与 llvm-mingw，并先按 [开发说明](../../docs/development.md#windows-构建) 安装 runtime-support，然后执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
target\windows\Velune\velune.exe
```

脚本输出 `target\windows\Velune`：先构建 `velune_bindings.dll`，再用 `velune-gobindgen` 从中生成 Go 绑定到 `target\bindings\go`（`go.mod` 以 `replace` 引用，生成物不入库），最后以 `CGO_ENABLED=0` 构建无控制台窗口的 `velune.exe`。输出目录包括 `velune.exe`、两个 DLL 以及 Harness 辅助脚本、`node_modules` 和第三方许可组成的 `resources`；整个目录可以移动，但各部分必须放在一起，Windows 从 exe 所在目录解析 DLL 依赖。

Go 部分可在 macOS 上交叉编译（`GOOS=windows GOARCH=amd64 CGO_ENABLED=0 go build`）。同一份代码也能在 macOS 上运行，只需把 `libvelune_bindings.dylib` 与 `resources` 放在可执行文件旁边，便于开发时检查。

## 数据与 Harness

应用数据位于 `VELUNE_HOME`，缺省为 `%USERPROFILE%\.velune`；远控身份与状态在其中的 `remote` 目录，设备名取自 `COMPUTERNAME`。

Harness 原生运行于 Windows，不经 WSL 或 SSH runtime。npm／pnpm 为每个命令生成 `.cmd`、`.ps1` 和无扩展名的 `sh` 脚本三种启动器。Pi 的安装解析只读取 `sh` 脚本里的目标路径，所以发现时使用这一个；Codex 与 Claude Code 直接启动二进制，优先 `.exe`，否则用 `.cmd`。Pi 需要 Node 的绝对路径，发现时取 PATH 上第一个 `node.exe`。核心把子进程放在 kill-on-close 的 Job Object 中，应用退出（包括被强制结束）时，Harness 及其子进程随之结束。

首次冷启动时，Windows Defender 扫描可能使 Pi 的版本探测超过 15 秒而失败，重试即可。

## 已知限制

- MyGO 的 Markdown 代码块在 Windows 上不显示制表符缩进（macOS 正常）。
- 双击依赖系统报告的点击次数，headless 检查无法模拟，需在真机上验收；中文输入法同样尚未在真机上验收。
- SSH 运行时依赖 OpenSSH ControlMaster 与 Unix socket，Windows 宿主不支持。
- 附件、会话管理（归档、删除）、资源管理、OAuth 订阅登录与用量页不在此 MVP 内，可在 Mac 上完成。
