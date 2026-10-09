# Mac 会话展示与导航

## 当前任务

2026-10-09 当前修复用户反馈的Markdown尾部点击重排、连续上滑跳动与长历史上滑卡住。上轮MarkdownView替换已安装至`/Applications/Velune.app`，源码`e1165e791afce77050e8dbcd7b217ef427e3c81b`，版本仍为`0.1 beta.1`；本轮尚未安装，尺寸／滚动判别见最后一节。

产品意图归[PRD](../../docs/prd/index.md)，跨单元契约归[architecture](../../docs/design/architecture.md)，平台行为归[Mac README](../../app/mac/README.md)。本任务也维护消息链接、大纲、可选CWD及独立原生Tabs。

## 授权与责任

用户授权替换、自主提交当前任务和重建安装Applications；不push。当前追加修复仅处理我们使用MarkdownView与原生列表的方式；用户进一步明确若定源为MarkdownView自身缺陷，不需要修上游。不新增自动化测试、不读取真实配置／凭据／会话、不调用真实提供商。使用静态检查、临时合成单实例端到端验收，真实体验由用户复验；临时GUI与目录finally清理。

session_management拥有SwiftPM依赖、消息缓存／渲染、许可、打包脚本及受影响manual入口，root整合文档／提交／安装。mac_navigation_advisor判断公开解析API、语言模式及资源打包边界。继续稳定owner，不撤销他人改动。

## 上轮实现与证据

固定MarkdownView3.0.0（6f452b55635246224a3329362e4e11cd3d592a30），官方tag支持macOS13+、Swift tools6.2，本机6.2.3满足。依赖与传递依赖许可证、字体／highlight.js声明随包分发，旧MarkdownUI／NetworkImage依赖和专用许可移除。使用上游包及公开API，不复制／修改上游源码，不留旧renderer兼容。

同步公开MarkdownReader首次提供MarkdownParseResult，结果随稳定消息对象保存，后续MarkdownView(result)跨native cell复用；离屏不主动解析，正文变化使缓存失效。解析配置当前固定，未来真实改变math／element renderer配置时需同步失效。统计名为cacheFillCount，不能当底层parser调用计数。5000离屏行0填充、首次20可见行20填充，100次未变化快照及host重建保持缓存／文档身份，单tail更新+1；稳定row与typed work消费通过。

原生UI实际通过060↔001／040远距跳转bubble y53、大纲原生双击、代码复制精确2行、失败link原文弹窗及复制。嵌套有序／无序列表、长代码＋表格跨屏STREAM END完整可见，历史阅读标题和气泡坐标在再次增高后不变；work开合三角在列内靠左，820↔1920宽窄行高正常。该轮未覆盖普通连续上滑和尾部重复点击；后续反馈显示仍需修正尺寸权威。

资源放app根实际不能strict签名。正式构建采用公开SwiftPM Xcode后端、host架构、标准Contents/Resources；其生成Bundle.module支持resourceURL，无源码／生成代码补丁。应用与生成绑定各自warnings-as-errors，依赖保持其声明的Swift5/6模式。真实frontend证据纠正了中间语言模式误判：Xcode失败来自第三方Swift5警告被全局提升。移开整个scratch后，独立ResourceProbe.app strict签名、实际Highlightr.highlight与SwiftMath默认字体加载均通过；manual资源入口finally恢复scratch，不借开发路径fallback。

布局阶段累计观察1753条AttributeGraph cycle告警，旧MarkdownUI table亦有同类304条setup告警。本轮最后独立idle时序被Mac锁屏阻断，0CPU／0log的未观察启动不能证明布局后停止；不宣称零告警或已测纯滚动FPS。关键GUI行为已通过。锁屏时的确证合成PID由owner立即终止并清理，不要求用户解锁。

## 验证与安装

Swift严格debug、cache手动脚本、Xcode完整debug、资源隔离签名／加载通过。改动manual脚本语法、shell语法及diff／相对文档链接检查通过；Rust源码未变。正式Xcode后端release构建、严格签名与安装通过。安装manifest指向上述源码且dirty=false，主二进制及Rust库SHA-256与签名构建包一致；标准Contents/Resources恰好包含Highlightr／SwiftMath两个所需bundle，根目录没有bundle；新锁文件与许可随包分发，旧渲染依赖／许可已移除。git diff／cached及相对链接检查通过，合成进程和固定临时目录均清理。源码替换提交e00bf98，许可空白格式规范提交e1165e7；不push。

## 已完成前置与边界

上一轮原生NSTableView修复已安装：完整user→nextuser范围折叠，只有真实末条纯LLM正文外露，生命周期决定状态／时长；可见行复用、实际列宽唯一高度约束、稳定阅读行及真实行内偏移恢复。系统背景、左对齐三角和outline双击已验。隔离Pi1.0.2七请求通过fullrange／systemtail、history重开／resume、原生确认缓存及Tab隔离；系统Pi公共安装悬空链接也已恢复，同版本隔离history读取exit0。真实提供商／既有会话未由开发工具读取。

网页／相对文件链接此前验到系统接收，未验外部查看器内容。DSH可选CWD已处理，实际CLI端到端尚未验。截图特定文字暗重未独立复现，用户报告上一轮改善／似乎消失。当前真实长期历史与实际提供商体验仍由用户验收。

## 当前修复：点击重排与连续上滑

用户反馈Markdown尾部2~3行点击后换行／行距变化，上滑跳动及长会话不能继续上滑。本轮Mac源码由session_management贯通修复，root负责文档、提交与安装，advisor负责尺寸权威与容器决定。transcript_projection只读核对后台成本。用户进一步限定仅修我们使用MarkdownView的方式，不修上游缺陷；上游源码未改。

click／scroll没有直接读取原生历史文件的路径，正常已同步Pi的idle poll消费内存快照。全量clone／投影／JSON传输有成本，合成release代理300／3000／12000行约1.18／16.42／68.82ms；不含真实FFI／UI且临时crate非生产locked依赖，不能归因用户视觉问题。本轮不扩poll契约。

已证缺口是自己的native接入：每次layout测高／写人工约束形成反馈；复用行在滚动条使列缩15pt后保留旧宽；高度变化及原生doc extent未提交就结束定位；尺寸变化后锚点离屏无法完成补偿；上滑结束nearBottom误恢复follow；work整组VStack破坏逐消息虚拟化。Store空Published数组无匹配removeAll仍发布，再触发workspace无条件复制／apply（独立Combine probe=1）。

冻结实现保留原生NSTableView负责行复用／滚动，关闭其automaticRowHeights。唯一实际显示NSHostingView提供intrinsic高度，heightOfRow只查稳定rowID、实际宽度、内容revision对应的真实尺寸，不进入SwiftUI求解。query拒绝旧宽／旧内容，回报拒绝旧generation；未物化行的原生44是暂态，不能用它宣布定位完成。实际root完成布局后才报告新版本，本地tool状态由同host layout事件传播，不伪造message revision。读intrinsic期间不再自排队，同高完成事件可推进pending但不提交高度变化。只有当前viewport和必需target／阅读anchor被物化，保留完整history。尺寸缓存写入前保存读锚点，批次及doc extent提交后定位；beginReadingInput立即清旧锚点／迟到jump并暂停follow，明确返回底部／发送才恢复。work展开是独立native成员行。

最终严格Swift warnings-as-errors、两manual脚本语法与diff检查通过。实际AppStore 100次无变化成功回调，application／workspace／transcript发布0／0／0；真实activity恢复、operation问题保留及配置修改仍传播。最终真实SwiftUI跳转state定位a119的top／clamp通过；末条增长后first239，32步真实beginReadingInput上移至row109／offset20，离屏尾部变化后仍109／20。宽→窄→宽→窄行框5163→6443→5163→6443；恢复follow后增长仍到底。tool39→670→39无需model revision；40work成员展开241／折叠201行。idle实际高度提交231、native intrinsic read1187保持稳定，本轮无布局／delegate重入告警。进程和临时目录均清理。入口为manual-transcript-layout.py及manual-store-idle.py，均显式手动，不接CI。

证据边界：早期NSTextView选择loop为空，因为此合成正文走SwiftUI Text；root已向用户撤销“8次选择通过”的说法。真实鼠标文字点击／物理滚轮及用户既有大history未验；不把离屏尺寸／原生入口操作当真实GUI手势通过。此前LLDB在run阶段45秒超时无有效stack，确证并终止唯一遗留synthetic子进程并删除副本；最终新接入未再出现该初始warning，不能据此承诺所有真实数据永远零告警。

## 当前容器与Textual评估

用户希望尽量采用现成原生或第三方虚拟列表。现有底层已为NSTableView，但我们自有尺寸／锚点／follow桥接的维护成本需要复核，不能因底层原生就宣布接入正确。旧List＋旧renderer证据不能排除List＋MarkdownView3。

advisor复核Textual0.5.0：MIT，tools6.0，macOS15／iOS18；用户随后已明确授权提高至macOS15，平台要求已同步PRD、SwiftPM与打包元数据。公开MarkupParser可接解析缓存，StructuredText默认仍是视图内整段parse缓存，选择有其自己的AppKit overlay，不能承诺直接解决列表问题。候选本身不授权立刻换包；原平台底线门槛已由用户独立决定解除。源码依据为tag manifest与公开实现，未复制源码。ListViewKit4.6.0有macOS12／Swift6.2／MIT，但自实现滚动及隐藏prototype测量，不优先于标准List；无LICENSE的AppKitScrollView不采用。

稳定Mac owner正在临时单变量比较SwiftUI List＋当前MarkdownView3，同一240行及实际消息组件，不改冻结产品source。首轮初始CPU1.04s／14cache填充，32步上移CPU4.25s；首跳成功，但尾部增高后scrollTo的真实state远跳5秒内偏255.7pt（body31325.7，clip31070），有一次delegate warning。marker／row rect oracle核对与cleanup尚待最终回报，不因候选失败扩回我们的桥接。root完成现有修复的提交／构建，安装状态稍后记录；不把当前组合永久固化。

List＋MarkdownView3隔离oracle核对完成：唯一长尾5139（其它最高984），偏255.7pt不是24pt inset，初次同布局精确。此候选不能在不新增定位桥接时完整替代已修复接入；子进程exit0仅正常记录失败／清理，不是候选PASS，后续width／tool／follow／idle未验。CPU口径含离屏layout／物化，非物理FPS；不外推Textual。冻结产品source未改，候选进程／tmp目录0。用户已允许macOS15，稳定owner现继续独立List＋Textual0.5.0单变量判别，使用包不复制源码，不先改产品依赖。
