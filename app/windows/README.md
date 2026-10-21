# Windows 完整端

Windows 的完整端：在本机运行 Agent、Harness 与网关，也能控制其他已配对的完整端。界面使用 [MyGO](https://github.com/egoist/mygo)（固定 `v0.5.5`）以纯 Go 自绘，这是原生视觉原则下用户接受的例外，交互仍尽量遵循 Windows 习惯。Go 经 `velune-bindings` 生成的 Go 绑定调用 Rust 核心：运行时用 purego 加载同目录的 `velune_bindings.dll`，不用 cgo，也不启动常驻 Host 进程（见 [bindings](../../packages/bindings/README.md)）。

此前的 GPUI 版已按用户 2026-10-20 的决定删除；进度与验证记录见 [任务](../../tasks/windows-mygo/packet.md)。

## 界面

界面结构以 Mac 为基准（见 [界面与交互](../../DESIGN.md)），外观用 MyGO 自身的组件与默认主题。

- **主窗口**：标题栏由界面绘制，保留系统窗口按钮：列表上方是设备切换（“本机”与已配对设备）和“新会话”（Ctrl+N）、“设置”（Ctrl+,）两个图标按钮，会话上方是会话标题。切换设备后界面不变，只换目标设备，本机与远端都通过同一个 `VeluneApplication` 访问。会话列表用 MyGO 的 Sidebar，与 Mac 相同：按核心给出的分组成节、可折叠（折叠状态存入该设备的列表偏好），每个会话一行，前面是未读圆点，后面是失败、等待处理或运行中；隐藏已归档会话与子 Agent。单击打开会话；双击、右键“重命名…”或 F2 就地编辑标题，Enter 保存，Esc 或焦点离开取消。未打开会话时显示品牌标志与新建入口，品牌图形取自构建复制到 `resources\Brand` 的 `assets/brand`。
- **新会话**：直接打开一个空会话，第一条消息发送时才创建；运行时与工作目录在输入区的操作行选择。
- **会话**：用户消息在右，LLM 在左，系统和 Harness 状态居中，不显示头像或昵称。LLM 正文按 Markdown 渲染，回复中途以流式呈现；思考和工具输出默认折叠，较大的内容在展开时读取。审批与提问显示在输入框上方。Ctrl+Enter 发送，运行中可停止。
- **输入区操作行**：与 Mac 相同的下拉。
  - 第一个标题为“运行时 · 模型 · 推理强度”，含三个子菜单：运行时（附“Agent 运行时设置…”）；模型（首项为“沿用当前（…）”或“运行时默认”，其后是运行时自己的模型与“Velune 提供商”一节，附“重新读取模型”和“AI 提供商设置…”）；推理强度（仅当所选模型声明级别时出现）。
  - 运行时类型声明权限模式时有第二个下拉，未选时沿用会话当前的模式。
  - 尚未发送的会话还有工作目录菜单：本机可选择目录，远端设备输入其上的绝对路径，另列最近使用的五个目录。
  - 下一轮可以换到另一个运行时，从当前会话的历史继续。选择按“会话 × 运行时”经核心保存；某运行时在此会话中还没有轮次与选择时，从该运行时上次的选择开始。
- **控制请求**：有设备请求控制本机时，主窗口弹出对话框，显示请求方名称和分组核对码。焦点落在核对码上，Enter 和 Esc 不触发任一按钮，必须点击“允许”或“拒绝”。
- **设置**：侧栏的设置按钮、Ctrl+,、操作行菜单中的设置入口，或设备切换菜单中的“添加设备…”打开设置窗口，只有一个，再次打开时回到已开的窗口。分类沿用 Mac 中 Windows 已有的三项，名称与顺序相同：AI提供商、Agent 运行时、设备。前两项是一个列表：单击选择，双击或“编辑…”在表单中编辑，右键菜单提供同样的操作，列表下方用＋／−增删，删除前确认（默认按钮为“取消”）。表单用 MyGO 的 Form／Field，必填项留空时在字段下方提示，保存失败显示在表单里。
  - **AI提供商**：列出本机网关的提供商（模型数与接口地址）。添加时可选 HTTP API key 模板或自定义 API，填写名称、协议、接口地址、API key 与以逗号分隔的模型 ID；编辑时保留的模型 ID 沿用原记录，API key 留空保持不变，“显示”读出已保存的 key 以便查看和修改。API key 保存在应用私有配置中；其他认证方式的提供商只能编辑名称、协议、地址与模型。ARK Coding Plan 另有与 Mac 相同的“额度查询认证”一组：AK/SK、本次关联确认与清除，和模型 API key 一起原子保存；留空保留，无查询认证时仍可先无上限调用模型。关联由用户确认，账户归属不自动核验；没有独立的额度页。
  - **Agent 运行时**：列出本机的运行时实例（类型、是否停用）。＋按类型手动添加（路径字段可浏览），“快速导入…”在 PATH、npm／pnpm 与厂商安装位置中发现 Pi、Codex、Claude Code，只读取公开版本；编辑可改名称、启用与类型的配置项，其他设置保持不变。ACP 类型的启动参数与环境不在这里编辑。
  - **设备**：与 Mac 相同的两组。“这台电脑”：允许其他设备控制的开关、设备名称、“添加控制设备…”（五分钟内有效、只能使用一次的邀请，以二维码与“复制邀请”给出，对方使用后或过期即收回），以及可以控制这台电脑的设备（撤销）。“控制其他设备”：已配对设备及其连接状态或核对码，“打开”在主窗口切换到该设备，配对中的可“取消”，右键菜单“移除此设备”；下方粘贴另一台设备的邀请并连接。

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

Harness 原生运行于 Windows，不经 WSL 或 SSH runtime。npm／pnpm 为每个命令生成 `.cmd`、`.ps1` 和无扩展名的 `sh` 脚本三种启动器。Pi 的安装解析只读取 `sh` 脚本里的目标路径，所以发现时使用这一个；Codex 与 Claude Code 直接启动二进制，优先 `.exe`，否则用 `.cmd`。Pi 需要 Node 的绝对路径，发现时取 PATH 上第一个 `node.exe`。PATH 之外，核心还列出厂商自己安装的位置：Codex 桌面应用（`%LOCALAPPDATA%\OpenAI\Codex\bin`）、OpenAI 独立安装器（Codex home 下的 `packages\standalone\releases`）、Claude 桌面应用（`%APPDATA%\Claude\claude-code`），以及 VS Code、VS Code Insiders 与 Cursor 中 Codex、Claude Code 扩展自带的可执行文件；隐藏的扩展暂存目录不算。是否可导入仍按适配器支持的版本判断。核心把子进程放在 kill-on-close 的 Job Object 中，应用退出（包括被强制结束）时，Harness 及其子进程随之结束。

首次冷启动时，Windows Defender 扫描可能使 Pi 的版本探测超过 15 秒而失败，重试即可。

## 已知限制

- MyGO 的 Markdown 代码块在 Windows 上不显示制表符缩进（macOS 正常）。
- 双击依赖系统报告的点击次数，headless 检查无法模拟，需在真机上验收；中文输入法同样尚未在真机上验收。
- SSH 运行时依赖 OpenSSH ControlMaster 与 Unix socket，Windows 宿主不支持。
- 附件、发送队列与插入本轮、ACP 会话选项、状态栏、编辑消息、会话管理（归档、删除）、资源管理、OAuth 订阅登录与用量页不在此 MVP 内，可在 Mac 上完成。模型目录读取中或失败时，模型菜单只说明状态，不像 Mac 那样先列出可达的 managed 模型。

## 第三方资源

`icons/` 中的图标来自 [Lucide](https://lucide.dev)（`lucide-static` v0.545.0，ISC 许可，见 [LICENSE](icons/LICENSE)），编译时嵌入 `velune.exe`，许可随构建复制到 `resources\ThirdParty\Lucide`。
