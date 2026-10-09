# Mac 会话展示与导航

## 当前任务

2026-10-09 用户复验：文字重影改善／似乎消失，但消息背景突然变白、工作过程折叠丢失，且消息列表严重卡顿。用户补充大纲项双击直接前往。本轮源码已冻结，隔离端到端与原生交互验收通过，正在最终构建安装。Applications仍是上一轮源码`25f2e3a`，待下方安装证据更新。

本任务也维护网页／本地文件链接与失败原文复制、outline定位、可选CWD、独立原生Tabs。产品意图归[PRD](../../docs/prd/index.md)，跨单元契约归[architecture](../../docs/design/architecture.md)，平台行为归[Mac README](../../app/mac/README.md)。

## 授权与责任

用户授权修复并自主提交当前任务，Mac修改后重新构建安装Applications；不push。不新增自动化测试、不读取真实配置／凭据／会话、不调用真实提供商。使用静态检查、合成loopback和单实例原生手动验收，用户执行真实产品验收。临时GUI及目录必须finally清理。

root整合、文档、提交及安装；transcript_projection负责Rust投影与桥接；session_management负责Mac消费、原生容器和手动验收；mac_navigation_advisor负责容器／尺寸归属判断；pi_install_repair负责系统公共安装。保持稳定owner，不撤销他人编辑。

## 根因、决定及证据

上一轮把第一条completed纯正文当作工作范围终点，混淆消息完成与Harness执行结束。旧源码隔离回归user→completed进度正文→tool→final得到work=[]，新源码得到[progress,tool]。现在完整拥有user→nextuser范围，仅真实末条纯正文外露（可流式），末条Tool／mixed／System全部归work；生命周期决定isRunning与时长。不新增phase契约。连续快照flatten验收全部canonical消息恰好一次、有序归属，覆盖progress→tool→stream→mixed→final→settle、身份确认、分支替换、空正文及未知历史。huihua历史生产者缺completed已补当前字段。

List还将DisclosureGroup当outline节点，导致三角出列／整表缩进，现消息折叠统一行内原生NSButton，设置DisclosureGroup不改。白底来自恢复List默认背景，现消息区域采用不透明系统windowBackgroundColor，不使用透明材质或固定灰白。

同一合成实例、160条长Markdown：List滚动＋AX观察阶段累计CPU约+95.65秒，Lazy同类阶段约+1.23秒。Lazy首次040↔001正确，但末条增高后060定位失准；最后完整根ID、唯一scrollPosition、无跟底竞争判别仍落在正文中部。因此采用AppKit单列NSTableView，稳定row-index物化／定位、可见hosting cell复用；不保留容器开关、不隐藏历史、不估高循环。

原生table严格debug构建通过，060↔001及大纲原生双击已正确定位／关闭sheet。一次CUA旧sheet绑定timeout经同PID sample确认mainthread正常mach_msg等待，重新绑定正常，非产品卡死。首版长正文增长行高未更新，正在以实际列宽sizeThatFits→唯一cell高度约束→尺寸改变才noteHeight修复；hosting自动尺寸约束禁用，0宽不测，不受旧高度限制。唯一高度权威版本已验STREAM END完整可见、work含长文与工具开合、宽→窄恢复、三角在消息列内且header左对齐、系统背景协调；GUI操作后idle CPU为0，无持续height循环。最终reader暂停、001↔060往返（bubble y53顶对齐）及用户大纲060展开／折回通过。

当前AppStore→UniFFI→隔离公共Pi1.0.2→loopback七请求全部通过，包括fullrange/systemtail、原生确认缓存、history重开、resume、duration和Tab隔离。5000消息缓存脚本证明100次同快照不重解析、单末条变化一次解析，并新增typed work消费断言；不能据此声称UI FPS。

系统Pi1.0.2入口目标包消失／pnpm链接悬空，非helper代码回归。按既有修复授权同版本公共包重装后，pi --version=1.0.2，SDK解析及隔离history helper exit0、sessions=[]。未读或改~/.pi、未调用模型。

## 验证与下一步

本轮cargo fmt/check/clippy workspace all-targets/all-features -D warnings、bindings no-default clippy、Node语法、四个改动manual脚本py_compile及diff检查通过；新release核心已供Mac联调，public绑定形状未变。接下来统一strict release构建、签名、提交安装、比对manifest/hashes及零GUI残留。

此前链接验到系统接收，未知scheme原文弹窗和复制一致，未验外部查看器内容。DSH可选CWD已处理，实际CLI端到端尚未验。截图特定暗重未独立复现，用户本轮报告改善／似乎消失。真实长期历史与实际提供商体验仍由用户复验，不读取其内容代替验收。

最终reader判别发现重分组结构reload后077阅读锚点退到074，不能报PASS。已保存变更前稳定可见行ID及真实row内偏移，替换后按原生索引恢复实际几何；最终PageUp阅读075标题y111及各段位置，末条增高同时结构重分组后仍逐项位置不变，不退到074、不拉底。最后合成PID正常退出exit0、目录和进程均为零；CPU累计18.91秒、idle0%。
