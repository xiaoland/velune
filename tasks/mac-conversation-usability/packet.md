# Mac 会话展示与导航

## 当前任务

2026-10-09 用户要求用LiYanan2004/MarkdownView替换MarkdownUI。接入与隔离验收已完成，正在提交、release构建及安装。Applications当前仍是上一轮`8df749b`原生消息列表修复版本`0.1 beta.1`；本轮安装后更新下方证据。

产品意图归[PRD](../../docs/prd/index.md)，跨单元契约归[architecture](../../docs/design/architecture.md)，平台行为归[Mac README](../../app/mac/README.md)。本任务也维护消息链接、大纲、可选CWD及独立原生Tabs。

## 授权与责任

用户授权替换、自主提交当前任务和重建安装Applications；不push。不新增自动化测试、不读取真实配置／凭据／会话、不调用真实提供商。使用静态检查、临时合成单实例端到端验收，真实体验由用户复验；临时GUI与目录finally清理。

session_management拥有SwiftPM依赖、消息缓存／渲染、许可、打包脚本及受影响manual入口，root整合文档／提交／安装。mac_navigation_advisor判断公开解析API、语言模式及资源打包边界。继续稳定owner，不撤销他人改动。

## 本轮实现与证据

固定MarkdownView3.0.0（6f452b55635246224a3329362e4e11cd3d592a30），官方tag支持macOS13+、Swift tools6.2，本机6.2.3满足。依赖与传递依赖许可证、字体／highlight.js声明随包分发，旧MarkdownUI／NetworkImage依赖和专用许可移除。使用上游包及公开API，不复制／修改上游源码，不留旧renderer兼容。

同步公开MarkdownReader首次提供MarkdownParseResult，结果随稳定消息对象保存，后续MarkdownView(result)跨native cell复用；离屏不主动解析，正文变化使缓存失效。解析配置当前固定，未来真实改变math／element renderer配置时需同步失效。统计名为cacheFillCount，不能当底层parser调用计数。5000离屏行0填充、首次20可见行20填充，100次未变化快照及host重建保持缓存／文档身份，单tail更新+1；稳定row与typed work消费通过。

原生UI实际通过060↔001／040远距跳转bubble y53、大纲原生双击、代码复制精确2行、失败link原文弹窗及复制。嵌套有序／无序列表、长代码＋表格跨屏STREAM END完整可见，历史阅读标题和气泡坐标在再次增高后不变；work开合三角在列内靠左，820↔1920宽窄行高正常。无需新增native尺寸适配。

资源放app根实际不能strict签名。正式构建采用公开SwiftPM Xcode后端、host架构、标准Contents/Resources；其生成Bundle.module支持resourceURL，无源码／生成代码补丁。应用与生成绑定各自warnings-as-errors，依赖保持其声明的Swift5/6模式。真实frontend证据纠正了中间语言模式误判：Xcode失败来自第三方Swift5警告被全局提升。移开整个scratch后，独立ResourceProbe.app strict签名、实际Highlightr.highlight与SwiftMath默认字体加载均通过；manual资源入口finally恢复scratch，不借开发路径fallback。

布局阶段累计观察1753条AttributeGraph cycle告警，旧MarkdownUI table亦有同类304条setup告警。本轮最后独立idle时序被Mac锁屏阻断，0CPU／0log的未观察启动不能证明布局后停止；不宣称零告警或已测纯滚动FPS。关键GUI行为已通过。锁屏时的确证合成PID由owner立即终止并清理，不要求用户解锁。

## 验证与安装

Swift严格debug、cache手动脚本、Xcode完整debug、资源隔离签名／加载通过。改动manual脚本语法、shell语法及diff／相对文档链接检查通过；Rust源码未变。下一步冻结源提交，执行正式release、严格签名、安装、manifest／二进制比对及零GUI残留。

## 已完成前置与边界

上一轮原生NSTableView修复已安装：完整user→nextuser范围折叠，只有真实末条纯LLM正文外露，生命周期决定状态／时长；可见行复用、实际列宽唯一高度约束、稳定阅读行及真实行内偏移恢复。系统背景、左对齐三角和outline双击已验。隔离Pi1.0.2七请求通过fullrange／systemtail、history重开／resume、原生确认缓存及Tab隔离；系统Pi公共安装悬空链接也已恢复，同版本隔离history读取exit0。真实提供商／既有会话未由开发工具读取。

网页／相对文件链接此前验到系统接收，未验外部查看器内容。DSH可选CWD已处理，实际CLI端到端尚未验。截图特定文字暗重未独立复现，用户报告上一轮改善／似乎消失。当前真实长期历史与实际提供商体验仍由用户验收。
