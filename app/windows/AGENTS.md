# Local Instructions: Windows app

> Applies to: `app/windows/**`

## Invariants and Authority

- 产品行为以 [PRD 分册](../../docs/prd/index.md#能力)为准，核心与 app 的分工见 [Product TDD](../../docs/product-tdd/index.md)；不从 Mac 源码反推产品行为。
- 界面结构以 Mac 为基准、控件用 WinUI 自带的 Fluent 控件（[DESIGN.md](../../DESIGN.md)）。

## Hazard and Tripwire

- **回调里的异常会结束进程。** `DispatcherQueue`、`DispatcherTimer`、事件处理和 `async void` 中抛出的异常成为 stowed exception，WinUI 以 0xC000027B fail-fast，常常不经过 `UnhandledException`，也可能不写 `crash.log`。
  - 症状：窗口突然消失，事件日志里没有 Application Error 或只有 0xC000027B。
  - 诊断：用 Silent Process Exit 监控（IFEO `GlobalFlag 0x200`）拿到退出码与触发方，用完删除注册表项。
- **一个元素只能有一个父级。** 把已挂在别处的元素加进 `Children`／`Content` 抛 0x800F1000（“element already has a parent”）。可复用的卡片或控件只构建一次并只挂一处；重建列表时新建子元素，不要搬移旧的。
- **窗口关闭后不再碰它。** 关闭后核心推送、计时器与异步续体仍可能到达；先在 `Closed` 里退订并停止计时器，回调开头检查窗口是否已关闭。
- **代码里不取固定主题的画刷。** `Application.Current.Resources[key]` 取到的画刷固定为一种主题，深色模式下显示错色。颜色写成 `App.xaml` 里带 `{ThemeResource}` setter 的样式，代码用 `Ui.Style(key)` 套用。
- **页面早于显示就存在。** 设置窗口一开就构建全部页面；计时器和轮询只在 `Loaded` 后启动、`Unloaded` 时停止。

## Required check

- 在 win-ws 上 `scripts\build-windows.ps1` 通过；改动涉及的界面在浅色与深色下各打开一次，跑足以触发计时器与核心推送的时长后正常退出（退出码 0，无 `crash.log`）。
