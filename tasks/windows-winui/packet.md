# Windows WinUI 应用

## 当前任务

2026-10-21 用户在 MyGO 版观感不佳后授权开发方自选 Windows 技术栈，除非找到积极维护的 GPUI 组件库，否则不再选 GPUI（[S45](../../docs/sources.md#s45)）。开发方选定 C#＋WinUI 3（Windows App SDK）。目标与 MyGO 版相同，即完整端：本机运行 agents，也能控制其他已配对设备。交互结构以 Mac 为基准，控件与主题用 WinUI 的 Fluent 控件。产品约束见 [PRD](../../docs/prd/index.md)，跨单元方案见 [架构](../../docs/design/architecture.md)，界面与构建见 [Windows README](../../app/windows/README.md)。用户随后要求继续，并使用 Velune 的标志包。

2026-10-21 用户要求把 Windows 端从 PoC 推进到 MVP，功能与 Mac 一致，指出至少缺工作过程折叠、与 Mac 类似的布局系统、完整的提供商功能（订阅导入、运行时导入、模板）、模型路由与大纲。产品行为从 durable docs 理解，不从 Mac 源码反推；PRD 因此按能力拆成分册，核心与 app 的分工写入 [Product TDD](../../docs/product-tdd/index.md)，WinUI 的易错点写入 [app/windows/AGENTS.md](../../app/windows/AGENTS.md)。结构名称（设置分类、菜单、布局分区）以 [Mac README](../../app/mac/README.md) 为基准，行为以 PRD 为准。

## MVP 差距与顺序

按 PRD 分册列出 Windows 尚缺的能力，括号内为核心已提供的接口；核心都已具备，缺的是 Windows 的呈现。按顺序推进，每步在 win-ws 构建并以 fixture home 验证后提交。

1. **消息展示**（[分册](../../docs/prd/conversation-view.md)；`TranscriptTurns`、`TranscriptOutline`）：“用户消息 – 工作过程 – 回复”折叠与时长，运行中默认展开，思考块以摘要为标题、无摘要不显示；工作过程行的轮次配置（悬停运行时与模型，点按完整配置与用量）；发送后本地行“发送中”；大纲（弹出面板／就地折叠）；回到底部；链接交给系统打开、相对路径按会话目录；附件缩略图；协作与子 Agent 的“打开会话”、来源会话标注、子 Agent 只读。
2. **设置“通用”**（[输入区](../../docs/prd/composer.md)；`set_usage_display`、`set_status_line`、`set_outline_presentation`、`set_linked_conversation_opening`、`set_conversation_browser_limits`）：用量显示、状态栏项、大纲按钮、关联会话打开方式、列表加载条数与总数上限；按 [S39](../../docs/sources.md#s39) 会话与快捷键并入通用（Mac README 仍分列，以 PRD 为准）。
3. **提供商**（[分册](../../docs/prd/ai-providers.md)；`provider_templates`、`authentication_*`、`preview_provider_import`／`apply_provider_import`、`save_model_template`、`fetch_public_model_catalog`、`provider_quota`）：按订阅／API 分组的模板与“从 Agent 运行时导入”、订阅披露、订阅登录与 Claude Code 账户来源、导入预览、提供商内模型的 model ID 与参数、模型模板与公开目录快填、额度与使用上限。
4. **模型路由**（[分册](../../docs/prd/model-routing.md)；`save_model_route`、`delete_model_route`）：设置页列出路由与成员现状，编辑名称、策略、会话保持、成员顺序与权重。
5. **输入区**（[分册](../../docs/prd/composer.md)；`composer_drafts`、`set_composer_draft`、`upload_attachment`、`send_queue`、`steer_turn`、`edit_message`、`acp_session_config`）：草稿持久化、附件、状态栏、发送队列与插入本轮、继续、编辑消息与 fork、ACP 会话选项。
6. **会话列表管理**（[分册](../../docs/prd/conversations.md)；`organize_conversations`、`set_project`、`search_conversations`、`delete_conversation`、`show_archived_conversations`）：“显示”菜单的分组、筛选、组排序（含手动）、会话排序，搜索，置顶、已读／未读、移到项目、归档、拷贝 ID 与链接，项目编辑，删除，多选，重新读取。
7. **布局系统**（[分册](../../docs/prd/workspace-layout.md)）：分屏标签组、左／右／底停靠区与工具窗口（会话列表、大纲、输入区、关联 Agent）、浮动与独立窗口、布局恢复、保持在最前面、快捷键。布局是 app 自有状态（Product TDD），动手前先写 Windows 的 Unit TDD。
8. **协作**（[分册](../../docs/prd/collaboration.md)；`collaborators`、`stop_sub_agents`）：关联 Agent 面板与打开方式。
9. **问题与通知**（[分册](../../docs/prd/diagnostics.md)）：集中的问题窗口；轮次结束与待确认的系统通知。
10. **运行时设置补全**（[分册](../../docs/prd/agent-runtimes.md)；`configure_runtime_execution`、`runtime_home_directory`、`upsert_acp_runtime`、`acp_runtime_info`）：HOME 归属、ACP 启动参数与 Agent 信息、新会话默认。SSH 运行位置在 Windows 宿主上不支持，列为已知限制。
11. **资源库与分析**（[资源库](../../docs/prd/resource-library.md)、[分析](../../docs/prd/analytics.md)）。

## MVP 进度

- 1–6 已实现，win-ws 构建零警告；呈现见 [Windows README](../../app/windows/README.md)。
  - 第 4 步：设置窗口增加“模型路由”页，位于 AI提供商与 Agent 运行时之间（Mac 的顺序）。成员现状与编辑合在页内，不分详情和编辑两处：Windows 设置的二级页面就是这种模式，窗口宽度也放不下左右分栏。成员顺序用上移、下移调整，不用拖动：ListView 拖动重排要移动作为条目的元素本身，这正是 [AGENTS.md](../../app/windows/AGENTS.md) 中“单一父元素”的易错点。
  - 第 5 步：“继续”发出的文字是“继续”，PRD 只要求它是一条真实的用户消息；编辑消息只发出文字，因为核心按本次的 turn 组装附件，原消息的附件不会自动再附上。
  - 第 6 步：侧栏仍是 NavigationView，多选与拖动分组随第 7 步把列表重建为分组的原生列表（“会话列表”工具窗口，先写 Unit TDD）时加入；组的手动排序先用右键菜单的上移、下移。NavigationView 的条目不支持多选，在其上拼凑多选会在第 7 步整个丢弃。项目文件夹的增删以对话框完成，与 Mac 的项目编辑同一组字段（核心 `set_project`）。
  - 第 7 步：先写了 [Unit TDD](../../app/windows/TDD.md)（布局模型、WinUI 映射、保存与快捷键、交付顺序）。其第 1 项与第 2 项的列表部分已实现：工作区窗口、分屏与 `TabView` 标签、三个停靠区与三个工具窗口、布局保存与恢复、命令表的默认快捷键，以及以 `ListView` 重建的会话列表（多选、打开方式）。`MainWindow` 由 `Shell`（应用级共享）与 `WorkspaceWindow` 取代。其余项（拖放、多窗口、浮动工具窗口、快捷键设置、关联会话的浮层与新窗口）待续。
  - **尚未截图验证**：fixture 的驱动只在用户空闲 10 分钟以上且 Velune 在前台时操作。这三次运行时 RDP 画面未显示，前台是外壳窗口（无主窗口的句柄），新启动的窗口拿不到前台，`SetForegroundWindow`（借 `AttachThreadInput`）也被拒绝，所以只验证了启动与退出（退出码 0），提供商、路由与列表管理没有实际操作过。
- 第 2 步的“关联会话打开方式”留到第 7 步：Windows 还没有标签页、浮层与多窗口，只能在当前视图打开（标题栏返回可回到上一个会话）。现在加入这项设置，所选的方式不会生效。
- 额度窗口的显示名由 Windows 按核心的窗口 ID 拼出（`window:<秒>`、`ark:*`、`kimi:*`）。各端都需要这套名称，候选做法是由核心随观测给出显示名（属 Product TDD 的“选项目录”）；改动涉及 bindings 与 Mac，未做。

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
- 用户反馈快速导入中各 Claude Code 2.1 都报“运行时目录不存在”。核实（win-ws）：发现按 `CLAUDE_CONFIG_DIR`，否则 `~\.claude`，与 Claude Code 的约定一致；这台机器没有 `~\.claude`（只有 2026-08 的 `~\.claude.json`；`AppData\Roaming\Claude` 是桌面应用自己的目录），所以搜寻本身没有缺陷。但核对时发现 Windows 上 Claude Code 历史完全读不出：历史 helper 是 Python，std 的 PATH 解析把引号当作带引号条目的开头，机器 PATH 里一条 `C:\Program Files\PowerShell\7"` 让其后所有条目（含用户的 Python）都找不到，helper 从未启动；`run_prepared` 又丢掉了传输错误码，失败只显示为 “history helper: Transport”。现按 Windows 的方式（每个 `;` 切分、忽略引号）找 `python.exe`，并保留错误码；修正后已存在与尚不存在的目录都能列出（0 条失败）。所以发现要求目录已存在比核心需要的更严；用户同意后已放宽：目录只须是绝对路径，已配置判断在目录不存在时按原路径比较。快速导入里有适配器但未能导入的候选现提供“手动添加…”，关闭导入对话框后打开预填类型、名称、入口与目录的添加表单。
- 用户报告深色模式显示错色、标题栏按钮与标题栏高度不一致。原因：代码从 `Application.Current.Resources` 取的画刷固定为启动时的主题；窗口要求 Tall 标题栏按钮（48 px），WinUI `TitleBar` 默认约 32 px。颜色改为 `App.xaml` 中带 `{ThemeResource}` setter 的样式，`TitleBar` 设为 48 px。深色复验（fixture home）：欢迎页、会话、设置三页颜色正确，按钮与标题栏同高，退出码 0。
- 侧栏的会话标题来自 Harness 列表（fixture 为“Synthetic ACP”），打开后的标题来自快照，两者不同是 fixture 数据所致，不是界面问题。

## 待验证

- 中文输入法、双击与右键等真实指针操作、长转录的虚拟化与流式更新的性能、Windows 11 上的 Mica，需要用户真机验收。
- 标志在任务栏与标题栏上的实际观感需用户确认。
- 用户从 RDP 会话启动的三次崩溃没有找到 `crash.log`（计划任务启动的复现里有），原因未查明。
