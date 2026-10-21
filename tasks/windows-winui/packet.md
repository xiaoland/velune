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

## 待验证

- 真机界面：win-ws 的启动检查（临时 home、合成 ACP 运行时与两个项目的三个会话，只在用户空闲 10 分钟以上时以 UI Automation 打开会话与设置并截图，不发送键鼠输入）尚未执行，用户一直在使用这台机器。
- 中文输入法、长转录的虚拟化与流式更新的性能、Windows 11 上的 Mica。
- 标志在任务栏与标题栏上的实际观感需用户确认。
