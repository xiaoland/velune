# Windows WinUI 应用

## 当前任务

2026-10-21 用户在 MyGO 版观感不佳后授权开发方自选 Windows 技术栈，除非找到积极维护的 GPUI 组件库，否则不再选 GPUI（[S45](../../docs/sources.md#s45)）。开发方选定 C#＋WinUI 3（Windows App SDK）。目标与 MyGO 版相同，即完整端：本机运行 agents，也能控制其他已配对设备。交互结构以 Mac 为基准，控件与主题用 WinUI 的 Fluent 控件。产品约束见 [PRD](../../docs/prd/index.md)，跨单元方案见 [架构](../../docs/design/architecture.md)，界面与构建见 [Windows README](../../app/windows/README.md)。用户随后要求继续，并使用 Velune 的标志包。

## 选型依据（2026-10-21 核实）

- win-ws 为 Windows 10 22H2（build 19045）。MyGO v0.5.5 的 Vibrancy（Mica）需要 Windows 11 22H2 以上，在这台机器上不可用。MyGO 的默认主题是它自己的跨平台风格，不是 Fluent。官方示例在 headless 下渲染出的观感与本应用一致。所以 MyGO 的问题不在用法，非原生的上限改不了。
- WinUI 3 在 Windows 10 上同样用 Fluent（Windows 11 样式）的控件；Mica 只在 Windows 11 上启用，Windows 10 回落为纯色背景。
- Windows App SDK 2.x（2.0 于 2026-04-29 发布，现为 2.5.1）仍支持 Windows 10 1809 以上（[2.0 发布说明](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-2-0)、[支持的系统](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/support)）。
- .NET 8 于 2026-11-10 停止支持，所以用 .NET 10 LTS（支持到 2028-11-14）；win-ws 用 winget 安装了 SDK 10.0.401。

## C# 绑定

- 核心用 UniFFI 0.32.2。NordSecurity `uniffi-bindgen-cs` 的发布版 v0.11.1 只支持 UniFFI 0.31；上游开放 PR #176 把它升级到 0.32（`cf0aa7a8fb59358a846708730a58a02bf206ac05`）。上游仓库按提交号可直接安装，不依赖 PR 作者的分支。构建脚本以 `cargo install --locked --root target\tools\uniffi-bindgen-cs` 安装该提交。
- 生成器的缺陷：带字段 enum 的变体生成为嵌套 record，变体字段与另一个变体同名时会与继承来的嵌套类型冲突，编译失败（`BindingMessageBlock` 的 `Reasoning.text`、`Notice.text`，`BindingResourceTemplateContent` 的 `Subagent.instructions`）。uniffi 0.32 的 rename 支持 `Enum.Variant.field` 路径，`uniffi.toml` 的 `[bindings.csharp.rename.velune_bindings]` 把它们改为 `text_value`／`instructions_value`，沿用生成器对“字段与本变体同名”时追加 `Value` 的约定；只影响 C#，不改 Rust API，也不必 fork 生成器。生成器文档里的 rename 表格式（`name = …`）与实现不符，实际是“crate 名 → 路径 = 新名”。
- 生成器在 cargo 之外运行 `cargo metadata`；win-ws 上不设 `CARGO` 时找不到 cargo（Mac 上可以），构建脚本设为 cargo 的绝对路径。
- Mac 冒烟（临时 home，release dylib）：未经手改的生成物编译通过；打开 `VeluneApplication`，`List` 返回 5 个运行时类型，`Snapshot` 不存在的会话抛出类型化的 `BindingException.Diagnostic`。

## 实现（2026-10-21）

- MyGO 版逐项移植为 `app/windows`（C#，约 3k 行）：`NavigationView` 侧栏与可折叠分组、`InfoBadge` 未读、就地重命名、`TitleBar`、虚拟化转录（`KeepLastItemInView`）、Markdig 渲染的 Markdown、`Expander` 思考与工具输出、审批与提问卡片、操作行的运行时／模型／推理强度／权限／工作目录菜单、配对对话框，以及设置窗口的提供商、运行时（含快速导入）与设备（QRCoder 二维码）三页。核心调用在线程池执行，结果回 UI 线程；远控状态在后台线程跟随观察者。
- 标志包：标题栏 small、欢迎页 medium（随明暗换 graphite／dark）；`scripts/render-windows-icon.ps1` 在构建时用 WPF 的路径解析从 small／medium／large 母版绘制 16–256 px 的 theme 配色 `.ico`，用作 exe 与窗口图标。SVG 从文件流载入（非打包应用不靠 `file://` URI）。
- 体积：引用 Windows App SDK 元包会带入约 40 MB 的 Windows ML（onnxruntime、DirectML），改为只引用 WinUI、Foundation、InteractiveExperiences 组件包并限定 .NET 卫星资源语言后，输出约 231 MB、1000 个文件。`PublishTrimmed` 因 Windows SDK 投影与 WinRT.Runtime 不是裁剪安全（IL2104）而失败，未启用。
- win-ws 构建：`TreatWarningsAsErrors` 下零警告；完整 `scripts/build-windows.ps1` 增量约 30 秒。
- MyGO 版、其 Lucide 图标与 MyGO skill，以及 bindings 的 `velune-gobindgen` 与仅它使用的依赖已删除。Rust 静态检查：`cargo fmt --check`、`cargo check --locked` 通过；clippy（`--all-features -D warnings`）在本机须以 `CARGO_HOME` 指向 registry 的真实目录运行（`~/.cargo/registry` 是链接，askama 经链接解析模板配置会失败，属本机环境），如此通过。

## 真机启动检查（2026-10-21，win-ws）

在交互会话里用计划任务启动，用户空闲 10 分钟以上才运行。用临时 home，内有合成 ACP 运行时与两个项目的三个会话。只用 UI Automation 的 Invoke／Select／WindowPattern 操作，不发送键鼠输入，每步截图。

- 首次启动即退出（0xC000027B）：非打包构建没有生成 `Velune.pri`，编译后的 XAML 无法载入。WinUI 模板为非打包应用设 `EnableMsixTooling`，补上后正常。WER 在这台机器上不留报告，所以应用把未处理异常写入 `%TEMP%\Velune\crash.log`。
- 关闭时同样崩溃。临时诊断构建记录 first-chance 异常后定位到：主窗口关闭后，远控状态更新与列表轮询仍调用 `Render`，设置已关闭窗口的 `Title`（`0x800710DD`，“The WinUI Desktop Window object has already been closed”）。CsWinRT 在 WinRT 边界把它转为 stowed exception，不经过 .NET 的未处理异常事件。现在主窗口关闭时停止轮询、退订 Hub、关闭会话与设置；先改为“不在 Closed 中调用 Exit”并不解决问题，已作为无效假设撤回。
- 修正后：窗口在前台且响应，标题栏与欢迎页显示标志包，分组可折叠，会话转录与操作行、设置三页（提供商、运行时、设备）渲染正常；依次关闭设置与主窗口，退出码 0，无残留进程。工作集约 182 MB，打开设置后约 217 MB。
- 顺带修正：内置的设置项跟随系统显示语言（win-ws 为英文），侧栏其余部分是应用自己的中文，所以设为“设置”（内容与自动化名称）；设备页说明文字原被截断，改为换行。
- 用户随后报告频繁崩溃：打开设置几秒后窗口消失（三次，均在点“快速导入”前后）。退出监视（Silent Process Exit）记录为 WER 服务以 0xC000027B 结束进程，且应用日志没有 Application Error 事件。临时 home 复现得到 `crash.log`：设备页在设置窗口创建时就建好，每次重绘（15 秒计时器或远控状态变化）把同一个邀请输入框移入新行，依赖 `Parent` 先摘除；第二次重绘时摘除未生效，`UIElementCollection.Add` 抛 0x800F1000（元素已有父元素），在计时器回调里成为 fail-fast。与“快速导入”只是时间上重合；核心的发现在无界面、无控制台及交互会话中各跑一次均正常。上面的启动检查只看了各页的首次渲染，没有覆盖重绘，结论过宽。现在邀请卡片只建一次，重绘只把它重新加入已清空的页面；计时器只在设备页显示时运行。修正后复现：快速导入 30 秒、设备页 35 秒（至少两次重绘）均正常，退出码 0，无 `crash.log`。
- 用户反馈快速导入中各 Claude Code 2.1 都报“运行时目录不存在”。核实（win-ws）：发现按 `CLAUDE_CONFIG_DIR`，否则 `~\.claude`，与 Claude Code 的约定一致；这台机器没有 `~\.claude`（只有 2026-08 的 `~\.claude.json`；`AppData\Roaming\Claude` 是桌面应用自己的目录），所以搜寻本身没有缺陷。但核对时发现 Windows 上 Claude Code 历史完全读不出：历史 helper 是 Python，std 的 PATH 解析把引号当作带引号条目的开头，机器 PATH 里一条 `C:\Program Files\PowerShell\7"` 让其后所有条目（含用户的 Python）都找不到，helper 从未启动；`run_prepared` 又丢掉了传输错误码，失败只显示为 “history helper: Transport”。现按 Windows 的方式（每个 `;` 切分、忽略引号）找 `python.exe`，并保留错误码；修正后已存在与尚不存在的目录都能列出（0 条失败）。所以发现要求目录已存在比核心需要的更严，待用户决定是否放宽。快速导入里有适配器但未能导入的候选现提供“手动添加…”，关闭导入对话框后打开预填类型、名称、入口与目录的添加表单。
- 侧栏的会话标题来自 Harness 列表（fixture 为“Synthetic ACP”），打开后的标题来自快照，两者不同是 fixture 数据所致，不是界面问题。

## 待验证

- 中文输入法、双击与右键等真实指针操作、长转录的虚拟化与流式更新的性能、Windows 11 上的 Mica，需要用户真机验收。
- 标志在任务栏与标题栏上的实际观感需用户确认。
- 用户从 RDP 会话启动的三次崩溃没有找到 `crash.log`（计划任务启动的复现里有），原因未查明。
