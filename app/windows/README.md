# Windows 完整端

Windows 的完整端：在本机运行 Agent、Harness 与网关，也能控制其他已配对的完整端。界面用 C#（.NET 10）与 WinUI 3（Windows App SDK 2.5，非打包、自包含），控件与主题都是 WinUI 自己的 Fluent 控件，随系统明暗、强调色、对比度与文字缩放；Windows 11 上窗口用 Mica 背景，Windows 10 上为纯色。C# 经 `uniffi-bindgen-cs` 生成的绑定以 P/Invoke 调用同目录的 `velune_bindings.dll`，与核心同进程，不启动常驻 Host 进程（见 [bindings](../../packages/bindings/README.md)）。

2026-10-21 起取代 MyGO 版（[S45](../../docs/sources.md#s45)）；进度与验证记录见 [任务](../../tasks/windows-winui/packet.md)。

## 界面

界面结构以 Mac 为基准（见 [界面与交互](../../DESIGN.md)），控件用 WinUI 的现成组件。

- **主窗口**：标题栏用 WinUI 的 TitleBar，显示 Velune 标志、应用名与当前设备，左侧按钮收起或展开会话列表。会话列表是 NavigationView 的侧栏：顶部是设备切换（“本机”与已配对设备，另有“添加设备…”）和“新会话”（Ctrl+N），底部是“设置”（Ctrl+,）。切换设备后界面不变，只换目标设备，本机与远端都通过同一个 `VeluneApplication` 访问。列表与 Mac 相同：按核心给出的分组成节、可展开收起（状态存入该设备的列表偏好），每个会话一项，未读时带圆点徽标，后面是失败、等待处理或运行中；隐藏已归档会话与子 Agent。单击打开会话；双击、右键“重命名…”或 F2 就地编辑标题，Enter 保存，Esc 或焦点离开取消。未打开会话时显示品牌标志与新建入口。
- **新会话**：直接打开一个空会话，第一条消息发送时才创建；运行时与工作目录在输入区的操作行选择。
- **会话**：会话标题在内容区顶部。用户消息在右，LLM 在左，系统和 Harness 状态居中，不显示头像或昵称。转录是虚拟化列表，新内容到达时停在末尾。LLM 正文按 Markdown 渲染（Markdig 解析，WinUI 文本元素呈现，代码块等宽、可横向滚动），回复中途以流式呈现；思考和工具输出放在默认收起的 Expander 中，较大的内容在展开时读取。审批与提问显示在输入框上方。Ctrl+Enter 发送，运行中可停止。
- **输入区操作行**：与 Mac 相同的下拉。
  - 第一个标题为“运行时 · 模型 · 推理强度”，含三个子菜单：运行时（附“Agent 运行时设置…”）；模型（首项为“沿用当前（…）”或“运行时默认”，其后是运行时自己的模型与“Velune 提供商”一节，附“重新读取模型”和“AI 提供商设置…”）；推理强度（仅当所选模型声明级别时出现）。
  - 运行时类型声明权限模式时有第二个下拉，未选时沿用会话当前的模式。
  - 尚未发送的会话还有工作目录菜单：本机可选择目录，远端设备输入其上的绝对路径，另列最近使用的五个目录。
  - 下一轮可以换到另一个运行时，从当前会话的历史继续。选择按“会话 × 运行时”经核心保存；某运行时在此会话中还没有轮次与选择时，从该运行时上次的选择开始。
- **控制请求**：有设备请求控制本机时，主窗口弹出对话框，显示请求方名称和分组核对码。焦点落在核对码上，没有默认按钮，Esc 也不关闭，必须点击“允许”或“拒绝”。
- **设置**：只有一个设置窗口，再次打开时回到已开的窗口。左侧导航的分类沿用 Mac 中 Windows 已有的三项，名称与顺序相同：AI提供商、Agent 运行时、设备。前两项是一个列表：单击选择，双击、Enter 或“编辑…”在对话框中编辑，右键菜单提供同样的操作，列表下方用＋／−增删，删除前确认（默认按钮为“取消”）。必填项留空时在字段下方提示，保存失败显示在对话框里且不关闭。
  - **AI提供商**：列出本机网关的提供商（模型数与接口地址）。添加时可选 HTTP API key 模板或自定义 API，填写名称、协议、接口地址、API key 与以逗号分隔的模型 ID；编辑时保留的模型 ID 沿用原记录，API key 留空保持不变，“显示”读出已保存的 key 以便查看和修改。API key 保存在应用私有配置中；其他认证方式的提供商只能编辑名称、协议、地址与模型。ARK Coding Plan 另有与 Mac 相同的“额度查询认证”一组：AK/SK、本次关联确认与清除，和模型 API key 一起原子保存；留空保留，无查询认证时仍可先无上限调用模型。关联由用户确认，账户归属不自动核验；没有独立的额度页。
  - **Agent 运行时**：列出本机的运行时实例（类型、是否停用）。＋按类型手动添加（路径字段可浏览），“快速导入…”在 PATH、npm／pnpm 与厂商安装位置中发现 Pi、Codex、Claude Code，只读取公开版本；编辑可改名称、启用与类型的配置项，其他设置保持不变。ACP 类型的启动参数与环境不在这里编辑。
  - **设备**：与 Mac 相同的两组，每项是一张设置卡片。“这台电脑”：允许其他设备控制的开关、设备名称、“添加控制设备…”（五分钟内有效、只能使用一次的邀请，以二维码与“复制邀请”给出，对方使用后或过期即收回），以及可以控制这台电脑的设备（撤销）。“控制其他设备”：已配对设备及其连接状态或核对码，“打开”在主窗口切换到该设备，配对中的可“取消”，右键菜单“移除此设备”；下方粘贴另一台设备的邀请并连接。

## 品牌

`assets/brand` 的标志包随构建复制到 `Assets\Brand`：标题栏用 small 光学母版，欢迎页用 medium，浅色主题用 graphite、深色用 dark 配色。exe 与窗口图标由 `scripts/render-windows-icon.ps1` 在构建时从同一标志包绘制：16–24 px 用 small、32–64 px 用 medium、256 px 用 large，均为 theme 配色、透明背景、无底板（Windows 图标不加底板，theme 色在浅色与深色任务栏上都可辨认），不改动路径几何。

## 构建与运行

需要 Rust（MSVC 目标）、Visual Studio Build Tools、.NET 10 SDK、Go 与 llvm-mingw（后两者用于 RelayKit），并先按 [开发说明](../../docs/development.md#windows-构建) 安装 runtime-support，然后执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
target\windows\Velune\Velune.exe
```

脚本输出 `target\windows\Velune`：先构建 `velune_bindings.dll`，再用固定提交的 `uniffi-bindgen-cs`（安装在 `target\tools`）从中生成 C# 绑定到 `target\bindings\cs`（生成物不入库），最后 `dotnet publish` 出自包含的 `Velune.exe`。输出目录包括 .NET 与 Windows App SDK 运行时、两个 DLL，以及 Harness 辅助脚本、`node_modules` 和第三方许可组成的 `resources`，约 230 MB；整个目录可以移动，但各部分必须放在一起，Windows 从 exe 所在目录解析 DLL 依赖。生成绑定后也可以只用 `dotnet build app\windows\Velune.csproj` 编译。

## 数据与 Harness

应用数据位于 `VELUNE_HOME`，缺省为 `%USERPROFILE%\.velune`；远控身份与状态在其中的 `remote` 目录，设备名取自 `COMPUTERNAME`。未处理的异常记录在 `%TEMP%\Velune\crash.log`：WinUI 结束进程时只留下 stowed exception 代码（0xC000027B）。

Harness 原生运行于 Windows，不经 WSL 或 SSH runtime。npm／pnpm 为每个命令生成 `.cmd`、`.ps1` 和无扩展名的 `sh` 脚本三种启动器。Pi 的安装解析只读取 `sh` 脚本里的目标路径，所以发现时使用这一个；Codex 与 Claude Code 直接启动二进制，优先 `.exe`，否则用 `.cmd`。Pi 需要 Node 的绝对路径，发现时取 PATH 上第一个 `node.exe`。PATH 之外，核心还列出厂商自己安装的位置：Codex 桌面应用（`%LOCALAPPDATA%\OpenAI\Codex\bin`）、OpenAI 独立安装器（Codex home 下的 `packages\standalone\releases`）、Claude 桌面应用（`%APPDATA%\Claude\claude-code`），以及 VS Code、VS Code Insiders 与 Cursor 中 Codex、Claude Code 扩展自带的可执行文件；隐藏的扩展暂存目录不算。是否可导入仍按适配器支持的版本判断。核心把子进程放在 kill-on-close 的 Job Object 中，应用退出（包括被强制结束）时，Harness 及其子进程随之结束。

首次冷启动时，Windows Defender 扫描可能使 Pi 的版本探测超过 15 秒而失败，重试即可。

## 已知限制

- 中文输入法、长转录与长列表的性能尚未在真机上验收。
- SSH 运行时依赖 OpenSSH ControlMaster 与 Unix socket，Windows 宿主不支持。
- 附件、发送队列与插入本轮、ACP 会话选项、状态栏、编辑消息、会话管理（归档、删除）、资源管理、OAuth 订阅登录与用量页不在此 MVP 内，可在 Mac 上完成。模型目录读取中或失败时，模型菜单只说明状态，不像 Mac 那样先列出可达的 managed 模型。

## 第三方组件

NuGet 依赖：Windows App SDK（WinUI、Foundation、InteractiveExperiences）、Markdig（BSD-2-Clause）、QRCoder（MIT），随 `dotnet publish` 进入输出目录。
