# Mac 会话展示与导航

## 当前任务

2026-10-09 用户反馈消息列表快速上滑时渲染跳动、长历史上滑卡住，并授权重写整个消息列表渲染实现（必要时含 agent runtime 的 UI 事件投影），可自由本地提交，有余力时继续优化列表与交互体验。用户同时建议可考虑更换 Markdown 框架（如 Textual）。上一轮 MarkdownView＋NSHostingView 修复安装源码为 `4c5f4ed`；本轮重写见下文，版本仍为 `0.1 beta.1`。

产品意图归[PRD](../../docs/prd/index.md)，跨单元契约归[architecture](../../docs/design/architecture.md)，平台行为归[Mac README](../../app/mac/README.md)。本任务也维护消息链接、大纲、可选CWD及独立原生Tabs。

## 授权与责任

用户授权重写消息列表渲染、自主本地提交与重建安装 Applications；不 push。不新增自动化测试、不读取真实配置／凭据／会话、不调用真实提供商。使用静态检查、临时合成验证，真实体验由用户复验。

## 根因

旧实现中每行由 NSHostingView 承载 SwiftUI MarkdownView，行高只能在该 host 实际布局后异步回报。未物化行先用 44pt 暂态高度，快速上滑时新行陆续回报真实高度，锚点补偿在多个异步轮次中追赶，表现为跳动；长历史中测量、提交与锚点恢复相互排队，表现为卡住。问题属于“高度在显示后才知道”的结构，而非某一处参数。

## 方案（已实施）

- 渲染：swift-markdown（原 MarkdownView 的传递依赖，同一锁定版本）解析为 TextKit 1 属性文本；围栏代码为独立布局元素。移除 MarkdownView、Highlightr、RichText、SwiftMath 及资源包；此前未启用数学公式与语法高亮，无可见能力损失。
- 几何：每行布局是纯函数（内容修订、局部展开、列宽 → 元素帧与高度），测量与显示使用相同 TextKit 配置。可见带同步测量，其余后台测量，按距阅读位置排序，提交时同步补偿锚点。
- Textual 评估：其 StructuredText 仍是 SwiftUI 视图，行高同样在显示后才可知，且此前 List＋Textual 0.5.0 远跳判别偏差 287pt 未收敛；因此不作为本轮渲染器。若以后需要其视觉，可在本布局层之上作为渲染来源复核。
- 轮询：快照修订比较与绑定映射移到后台请求队列，未变化不映射。Rust 投影本轮未改；全量快照跨语言传输仍是已知成本。

## 判别过程要点

- 文本视图首次获得 layer 时 AppKit 会按像素对齐帧，分数 x/y 使宽或高 +1pt；改为整数帧并在创建时即 layer-backed。
- NSTableView 会延迟在其 clip view bounds 变化通知内提交的行高，导致高度在之后无补偿落地（实测锚点行位移约 7450pt）；滚动／宽度引发的测量改为下一主队列轮次。
- TextKit 以块自身背景绘制其 margin，且相邻同样式块会被合并；代码块因此改为独立元素，引用／表格间距按不绘制背景的一侧放置。
- fullWidth 表格样式在列外附加内边距，列宽须由 sizeLastColumnToFit 在视口宽度内求得，否则表格宽于视口。

## 验证

- `scripts/manual-transcript-scroll.py`（显式手动，不接 CI）：编译实际 transcript 源，800 行合成历史。加载后立即远跳未测量历史、约 980 步每步 260pt 快速上滑穿过未测量区：阅读行最大位移 <1pt（像素取整）；显示 TextKit 高度与测量帧一致、表格不宽于视口；宽窄变化、阅读时尾部增长、局部展开、远跳、回到底部与流式跟随均通过。`--snapshot` 输出视口 PNG 用于目视核对 Markdown 渲染。
- `manual-transcript-outline.py`（实际 AppStore＋隔离 Pi 1.0.2＋合成上游）通过，覆盖新后台轮询映射；缓存断言改为内容修订不变。`manual-store-idle.py` 通过。
- Swift warnings-as-errors debug 与 Xcode 后端 release 构建通过；preview 模式窗口截图核对版式。Rust 源码本轮未改。
- 边界：合成探针中个别步骤偶发 40–200ms 停顿，插桩显示本程序主线程工作 0–3ms，停顿不在列表代码内，未进一步归因；首次显示约 0.2–0.5s 主要为 CoreText 首次初始化。真实物理滚轮、真实长历史与提供商体验未由开发方验证。

## 构建与安装

2026-10-10 主工作区含其他会话未提交的 Rust／文档改动，为避免打包他人未完成工作，从源码 `6c764bb` 的独立干净 worktree 完成 locked Rust release 与 Xcode SwiftPM release 构建、严格签名，并安装至 `/Applications/Velune.app`（版本 `0.1 beta.1`）。安装 manifest source 一致且 dirty=false；主二进制与 Rust 库 SHA-256 与签名构建包一致；包内不再有资源 bundle。安装前正常退出运行中的 Velune，安装后重新打开。不 push。

## 已完成前置与边界

完整 user→nextuser 范围折叠，只有真实末条纯 LLM 正文外露，生命周期决定状态／时长；系统背景、左对齐三角和 outline 双击已验。网页／相对文件链接此前验到系统接收，未验外部查看器内容。DSH 可选 CWD 已处理，实际 CLI 端到端尚未验。真实长期历史与实际提供商体验仍由用户验收。
