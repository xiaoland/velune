# 来源：远程控制与平台选择

[返回来源索引](../sources.md)。本页保留原始决定与当时的授权边界；它不是当前实现状态或本次任务的授权清单。现行规则从[文档入口](../index.md)按主题查阅。

## S40

2026-10-17 用户要求参考 [liguobao/ds-harness-remote](https://github.com/liguobao/ds-harness-remote)，共同梳理远程控制需求、厘清架构后实现 Rust packages。随后明确有 GUI 与无 GUI 两种宿主都需要，应用装配相同共享核心，不能按第一批界面形态缩窄远控架构；公网使用不依赖预配 VPN 或端口转发。远控范围为完整 Velune 业务能力，信任对象为同一个人的多台设备。产品归 [PRD](../prd/remote-access.md)，方案与证据归 远控任务（git 历史：tasks/remote-control）。

用户随后选择“继续执行，其他已配对设备可接续操作”及“最终开箱即用，同时允许自部署”，确认控制端断线不终止执行、保留审批，以及公网服务的最终产品方向；本轮仍不部署服务。

用户随后明确同意使用 iroh，并要求继续参考、对比 ds-harness-remote 的 WebRTC／TURN 思路，以尽可能在无需中继的情况下获得稳定远控；用户认为该项目的实际效果较好。该体验反馈不替代 Velune 的直连／跨网络验收，也不应以最小自部署 Server 的能力边界否定上游客户端 P2P 机制。

用户在 Android 验收中要求暂停，重新讨论公共设施与繁琐的邀请粘贴。澄清后明确接受使用 iroh 公共服务，允许用户切换自部署或其它兼容协议的公开 relay，并纠正：“不应该让公网远程控制和局域网远程控制变成两套体验。”

用户随后补充“多路连接，也就是在公网中和进入同一局域网内，自动选择最优路径”，将统一体验延伸为已建立连接的路径发现和自动优选，而非只统一添加入口。

## S41

2026-10-17 用户在远控 Rust SDK 交付后要求：“那么我们现在来实现一个最小的安卓端？（我建议基于 kotlin, jet compose）”，并说明 adb 已连接同一 Wi‑Fi 内的设备，可用于验收、实验。产品归 [PRD](../prd/remote-access.md)，实现与真机证据归 Android 任务（git 历史：tasks/android-minimal）。该授权包含最小原生端的必要实现和设备实验，不包含真实模型调用、读取个人资料或公网服务部署。

暂停后的讨论中，用户认可“手机扫 Mac 二维码 → Mac 确认新设备 → 此后记住设备并自动重连”。该答复确认配对交互方向，没有回答是否必须支持无法操作执行设备的异地首次添加，也没有授权引入账户系统。

2026-10-18 用户要求“继续直到技术方案，验收方案，以及线性化实现计划收敛”。该轮继续设计与计划，产品实现／真机操作的暂停尚未解除；产出分别归 [技术方案](../product-tdd/remote-access.md)、验收方案（git 历史：tasks/android-minimal） 和 线性计划（git 历史：tasks/android-minimal）。

上述方案收敛后，用户明确“好的，开始实现”，恢复按线性计划进行必要源码修改、构建和既有范围内的合成设备实验；无真实模型／个人资料访问、公网部署或远端发布的新增授权。

2026-10-18 用户明确纠正：“你的边界应该仅限于 Rust Library，怎么会延伸到 Mac App 呢。”当前任务因而收回平台接入和界面验收，只继续 Rust Library／UniFFI 与库级合成验证；该纠正不等于取消 Android 产品目标，也不授权为库验收继续修改或启动 Mac App。范围归 [PRD](../prd/remote-access.md)，实施与收回记录归 当前任务（git 历史：tasks/android-minimal）。

2026-10-18 用户授权将已完成的 Rust Library 工作整合 main；保留 main 现有业务，不包含功能分支中的平台 App 改动，不包含远端发布。整合记录归上述当前任务。

## S43

2026-10-19 用户要求实现远程控制系列能力的 MVP：Windows 端（可在 win-ws.localhost 验收）；Android 端保持既有技术栈并彻底重写，现有实现只是测试原型；“Android 端不是对等的端（未来的 iPad, web 也是），它只有‘控制’能力，实际的 agent runtime 并不在该端上运行；但是任何一端，都具备控制其它端的能力”；“ssh agent-runtime 和我们这里的‘远程控制’不是一个能力”。

开发方核对后指出 MyGO 自绘界面与纯 Go 无 cgo 的取舍。用户答复：MyGO 看起来不太合适，可以看看别的选择（例如 GPUI），也可以先试 MyGO；“Windows MVP 需要包含在 windows 上运行 agents；接受 MyGO 的 native UI 作为例外（GPUI 也是），因为 windows 的 .net, winUI 等开发体验太糟糕了”；Go 桥“坚守 purego”；认可“one interface, switch the target device”架构；Mac 同时作为被控端与控制端纳入 MVP；MVP 先做会话日常闭环，并说明“providers, resources 等是要跨端同步的”；接受使用 iroh 公共 relay；remote 库可以继续修改，甚至进一步拆分；授权完全控制已连接本机的 Android 手机，并使用 win-ws.localhost 等设备。产品归 [PRD](../prd/platforms.md)，实施归 远控 MVP 任务（git 历史：tasks/remote-control-mvp）。


2026-10-20 用户在 GPUI 版 Windows 应用跨端验收后改定：“现在我们来转移一下阵线：MyGO Windows（我还是希望能使用 MyGO 而不是 GPUI）”。此前 spike 选 GPUI 的结论由此取代；Go 桥仍按上文坚持 purego、不用 cgo。实施归 Windows MyGO 任务（git 历史：tasks/windows-mygo）。

2026-10-20 开发方指出 MyGO 版界面仍是照 GPUI 版逐行移植、未用 MyGO 高层组件，且设置分类、模型菜单与布局和 Mac 不一致。用户答复：希望“尽可能贴合框架、平台的体验”，并“希望跨端的交互体验能尽可能一致（比如设置项目、布局系统、模型菜单选项）”；随后说明“贴近框架/平台”指贴近 MyGO 的风格即可，不必另做 Windows 外观；“安卓的差异很多时候是合理的，因为其屏幕尺寸、设备操作方式就存在巨大差距，也不应该照搬”；并确认“windows以mac为参考基准”。产品归 [PRD](../prd/platforms.md)。

## S45

2026-10-21 用户看过 MyGO 版主窗口后问：“这真的是 MyGO 重写的，并且使用了 MyGO Native UI 以及相关组件之后的效果吗？为什么和它们的官方 demo 还有我在推特上看到的别人的分享效果差这么远？是因为它对 windows 的支持不佳，还是你对它的了解不够（有 agent skill 的话建议安装）。如果这两个问题任何一个都无法改善，那我就放手交给你去选择了（不能再选 GPUI，除非你有找到一个还不错的有在积极维护的 GPUI 组件库），用你自己喜欢的不论是 C# 也好、.Net 也好。”

开发方核对：win-ws 为 Windows 10 22H2，MyGO 的 Vibrancy（Mica）需 Windows 11 22H2 以上；MyGO 的主题是其跨平台自有风格而非 Fluent，官方示例 headless 渲染与本应用观感一致，网上分享多为 macOS 的毛玻璃效果；已安装其 `mygo-maintenance` skill，内容只涉及生命周期与测试，不涉及视觉。界面用法已按高层组件改进，但非原生的上限不变。据此开发方在用户授权内选定 C#／.NET＋WinUI 3（实施时用 .NET 10 LTS，因 .NET 8 于 2026-11-10 停止支持）（Windows App SDK），经 UniFFI 的 C# 绑定调用 Rust 核心；本条取代 [S43](#s43) 中“不采用 .NET／WinUI”与 MyGO 的选择，Go／purego 约束随 MyGO 版一并失效。MyGO 版保留至 WinUI 版达到同等功能后删除。实施归 Windows WinUI 任务（git 历史：tasks/windows-winui）。

## S46

2026-10-23 用户：“本任务的核心目标是：改进 Android 端的使用体验（主要集中在 UI/UX 上）……目前安卓端的体验实在是远远低于平均线，所以请你先按照 lody/hapi 或者其它类似开源软件的安卓/iOS进行界面上的‘抄’（与此同时，即便没有直接使用它们的源代码，也需要增加相关的声明并确保我们的开源协议、法律条款等符合要求）”。即 Android 界面先以同类开源移动客户端为样板模仿其布局与交互，并附参考声明、核对许可义务；本条只就 Android 界面放宽 [S4](foundations.md#s4) 的“不照搬”，不改变其他能力的参考边界。开发方核对许可后只借鉴布局与交互，不复制 AGPL 项目（HAPI、lody-ios）的源码。产品归 [平台](../prd/platforms.md)，实施归 Android 体验任务（git 历史：tasks/android-ux）。

## S48

2026-10-23 用户：“最快速度开发一个最基础的 Velune iPad app（但最好未来也能直接构建为 iOS app），能力对齐 Android app 就好。注意避免过度实现、过度保护、验证。”授权以 Android 控制端为范围直接实现 iPad 优先、兼容 iPhone 的基础应用；不扩展本地 Agent 或完整端设置。实现归 [iOS 控制端](../../app/ios/README.md)，状态归 iPad 任务（git 历史：tasks/ipad-minimal）。

## S49

2026-10-25 用户要求 iPad 版本“现在目标不只是能用，还得好用、好看”，要求真实与视觉验收，同时明确“优先验收主要的路径和明显的问题，不要测试边界问题”。随后提出问题面板、多窗口、有效的已读未读、大纲、Agent 协作以及外接键盘下的高强度工作，要求由 PoC 迈入 MVP／MLP。在评审工作台范围后回复“开始”，授权实现与主路径验收。此决定扩展 [S48](#s48) 的能力范围，继续保留复用共享核心与现有库、避免过度实现的约束；不增加本地 Agent 或完整端设置。产品归 [平台](../prd/platforms.md)，实施与验收归 iPad 任务（git 历史：tasks/ipad-minimal）。

2026-10-26 用户要求临时安装适合当前技术栈的技能并全面、深入改善 iPad UI／UX，授权使用 SwiftUI Pro 和 SwiftUI UI Patterns 指导实现。随后强调应先从源码判断并修复明显问题，验收成本高，本轮允许模拟器验收；不再将等待真机或扩大测试范围作为推进条件。
