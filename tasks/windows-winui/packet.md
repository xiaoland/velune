# Windows WinUI 应用

## 当前任务

2026-10-21 用户在 MyGO 版观感不佳后授权开发方自选 Windows 技术栈，除非找到积极维护的 GPUI 组件库，否则不再选 GPUI（[S45](../../docs/sources.md#s45)）。开发方选定 C#／.NET 8＋WinUI 3（Windows App SDK）。目标与 MyGO 版相同，即完整端：本机运行 agents，也能控制其他已配对设备。交互结构以 Mac 为基准，控件与主题用 WinUI 的 Fluent 控件。产品约束见 [PRD](../../docs/prd/index.md)，跨单元方案见 [架构](../../docs/design/architecture.md)。

MyGO 版（`app/windows`，[任务](../windows-mygo/packet.md)）保留到 WinUI 版功能相同，然后与 `velune-gobindgen` 一并删除。

## 选型依据（2026-10-21 核实）

- win-ws 为 Windows 10 22H2。MyGO v0.5.5 的 Vibrancy（Mica）需要 Windows 11 22H2 以上，在这台机器上不可用。MyGO 的默认主题是它自己的跨平台风格，不是 Fluent。官方示例在 headless 下渲染出的观感与本应用一致。所以 MyGO 的问题不在用法，非原生的上限改不了。
- WinUI 3 在 Windows 10 上同样用 Fluent（Windows 11 样式）的控件。Mica 在 Windows 10 上回落为纯色背景，其余控件与主题资源不受影响。
- .NET 8 SDK 已装在 Mac（8.0.204）和 win-ws（8.0.426）上，win-ws 另有 VS 2022 Build Tools。

## C# 绑定 spike（Mac，临时程序未入库）

- 核心用 UniFFI 0.32.2。NordSecurity `uniffi-bindgen-cs` 的发布版 v0.11.1 只支持 UniFFI 0.31。上游的开放 PR #176 把它升级到 0.32（head `cf0aa7a8fb59358a846708730a58a02bf206ac05`，标为 "v0.12.0+v0.32.0"），可以合并，只差 DCO 签署。
- 用该 PR 版本生成 bindings 的 C# 代码，约 2.8 万行。.NET 8 控制台程序（`AllowUnsafeBlocks`）加载 release dylib，在临时 home 中打开 `VeluneApplication`，`List` 返回 5 个运行时类型，`Snapshot` 不存在的会话时抛出类型化的 `BindingException.Diagnostic`。record、enum、error、object 与集合都可用。
- 生成器有一个缺陷：带字段 enum 的变体会生成为嵌套类。若某个变体的字段与另一个变体同名，字段就会与继承来的嵌套类型冲突，编译失败。现有冲突是 `BindingMessageBlock.Text` 与 `Reasoning`／`Notice` 的 `Text` 字段，以及 `BindingResourceTemplateContent.Instructions` 与 `Subagent` 的 `Instructions` 字段。spike 中临时改名绕过；正式修复放在生成器里，不改 Rust API 的命名。

## 方案

1. **C# 绑定**：bindings unit 生成 C# 代码，不入库，与 Swift、Kotlin、Go 相同。候选：
   - 固定 PR #176 的提交，修补冲突后构建生成器，修补可提交上游（提交上游需用户授权）。
   - 仿照 `velune-gobindgen` 自写 C# 生成器，只覆盖实际用到的类型。

   倾向前者，因为生成代码完整、维护跟随上游。
2. **应用骨架**：`app/windows-winui`，非打包（unpackaged）、self-contained 的 WinUI 3 应用，用 dotnet CLI 在 win-ws 上构建。exe 旁放 `velune_bindings.dll`、`velune_relaykit.dll` 与 `resources`，构建脚本沿用 `scripts/build-windows.ps1` 的输出约定。
3. **主窗口**：标题栏、NavigationView 或 ListView 分组侧栏（设备切换、新会话、设置）、转录、输入区操作行、审批与提问，与 MyGO 版和 Mac 同构。
4. **设置与设备**：AI提供商、Agent 运行时、设备三类，与 Mac 相同。
5. 达到同等功能后删除 MyGO 版与 Go 生成器，并更新 README、开发说明与 bindings 文档。应用图标与桌面快捷方式随新 exe 重做。

## 待验证

- Windows App SDK 当前稳定版本及其在 Windows 10 22H2 上的非打包部署（引导程序或 self-contained）。
- 中文输入法、长转录的虚拟化与流式更新的性能。
- 观察者阻塞的 `next(timeout)` 放在后台线程，经 DispatcherQueue 回到 UI 线程。
