# 输入区与 Pi 发送修复

当前进展（2026-10-08）：最新重试为extension provider注册被一刀切拒绝导致exit1。已允许普通注册并将执行约束放在Pi公开streamSimple入口，合成6请求／续接、直连模型拒绝和保留名称冲突检查通过。新版已安装重新打开；root完成本地提交收尾。真实会话重试仍由用户验收。下文早期记录仅代表当时状态。

2026-10-07 用户报告 composer 占位文字与实际输入位置不一致，输入区过度复杂，并授权诊断修复连续两次 Pi 发送失败。目标是简洁的原生输入体验和可靠的发送链路，不扩展会话功能。真实凭据、配置与会话正文不得读取；仅查看对应失败诊断，行为验证使用临时 HOME 与合成本机服务，不调用真实模型。

root 负责诊断证据、整合、文档和重建安装；session_management 继续拥有 Mac composer 源码与界面隔离验证；pi_install_repair 拥有 Pi adapter／application 执行边界和必要人工脚本。各 owner 不交叉撤销改动。先静态检查和人工端到端检查，不新增自动化测试。

已观察到 UTC 11:41:25、11:41:31 的两次 send_turn 失败，操作编号分别为 18dc3c82abc60458-c166-8569、18dc3c840f207dc0-c166-8570。均发生在 runtime startup 的 readiness RPC，错误为 Pi RPC child exited before responding；这尚不能证明实际退出原因。Pi client 将 stderr 丢弃，与现有其他运行时有界错误输出的实现不一致。原始 stderr 不可事后补回，需要隔离复现并修复失败边界。

完成依据：占位与输入共享文本布局，常用输入／下一轮参数／发送操作层级清晰；实际外部 Pi 1.0.2 在隔离配置中通过发送路径，启动失败保留原因和实例非秘密定位字段；必要 Rust／Swift 静态检查及构建通过，重新安装 Mac app。真实调用与最终 UI 体验仍由用户验收。

advisor 的输入控件判别方案已采用：先用原生多行 TextField 的内置 prompt 和行数限制，删除独立 placeholder 几何、按显式换行估高、自绘重复边框及永久快捷键行。是否最终采用取决于实际 Return 换行、⌘Return 单次发送、长文本软换行／滚动和输入法行为；不通过 onSubmit(send) 或手工插入换行补偿。若原生控件不满足，回到固定合理高度 TextEditor，不先建 AppKit 测高体系。判断来自现有 composer 源码及 Apple 控件文档，真实键盘行为仍待 owner 验证。

实际 CUA 组件探针发现本机 TextField(axis:.vertical) 未遵守预期多行高度，Return 未插入换行；这推翻仅靠文档采用它的方案。owner 改为保留原生 TextEditor、固定合理编辑高度、清晰外部标签，删除不可靠的内部占位层及重复视觉包装；未引入自定义 NSTextView delegate 或测高层。

Pi 隔离复现结果：当前系统外部 Pi 1.0.2 在临时 HOME 与合成上游通过完整6请求流程，最小系统 PATH 下也通过；不能据此认定真实故障已修复，也不能把两次失败归因于版本或 GUI PATH。原始错误已被旧版丢弃，事后不可恢复。owner 已实现 stderr 有界持续 drain、退出状态和控制请求错误传播，合成退出17的错误能返回原始cause；root要求补齐 application 启动边界的实例／入口上下文并验证。真实退出原因需要新版中由用户重试后观察日志，不由开发方调用真实模型复现。

Mac 组件验收已采用：实际 ComposerView 临时原生窗口中 Return 插入换行，⌘Return 回调仅一次且包含完整多行文本；16行中英合成粘贴、软换行、原生滚动和撤销有效；生成时不发送，停止单次调用并保留草稿；纯空白不能发送。AX 输入标签与帮助有效，严格 Swift warnings-as-errors 构建通过。未切换用户系统输入源，IME 候选确认没有实测，不冒称通过。临时窗口及进程已正常退出清理。该证据为组件实际键盘／回调，不替代 Rust 发送链路。

整合验证完成：workspace cargo fmt/check/clippy（all-targets/all-features，-D warnings）通过，release Rust＋新生成 UniFFI＋Swift warnings-as-errors 整包构建及严格签名通过。扩展现有 manual-pi-native-loop.py，Pi helpers只复制到临时目录，先合成exit17 readiness失败，再恢复bundle实际helper，确认完整原因／实例／operation_id关联、失败前0请求及恢复后6请求流程通过；原来源合成文件字节不变。不修改系统Pi或真实配置，不新增自动测试。

当前已安装并重新打开标准 Applications 位置的0.1 beta.1；二进制、Rust动态库和manifest与通过人工验收的bundle一致，签名验证通过。当前改动未提交，manifest正确记录dirty=true，基线commit为dcdb067。输入区修复和诊断增强已交付；真实Pi退出原因仍待用户在新版重试，异步问题已发出，不将隔离发送成功记为真实故障修复。下一步只查看新重试对应的失败诊断，按原始cause继续修复，不读取真实凭据／会话正文。

用户随后纠正验收范围：composer 指会话详情底部整个面板，上轮仅调整文本框不足以改善它。当前继续同任务，Mac稳定owner负责完整底部面板与导航层级：运行时／模型改为紧凑直接选择，菜单保留设置入口，不各占半个全宽；新建会话与大纲移到detail右侧工具栏，筛选／分组继续sidebar工具栏。大纲跳转、用户消息展开模式和回到底部保持既有语义。先判断布局及交互，再实际观察整个detail区域，不用组件小窗口替代整体效果验收。root负责长期说明同步及安装，Pi诊断工作与待重试状态保持。

整块面板方案已采用：输入在上，runtime→model→留白→发送／停止的紧凑操作行在下；移除独立“下一轮”文字及两个菜单无限扩张。菜单直接列选项，末尾使用诚实的“设置…”入口，不声称原生 SettingsLink 能定向打开指定标签。下一轮语义通过help／AX保留；未选运行时可禁用模型选项，但不锁死设置入口。大纲显示状态上移为Root局部binding，sheet和proxy导航仍由Transcript负责，切换／加载时关闭旧大纲。不改Store或Rust契约。advisor要求的判别证据是完整NavigationSplitView、长名称、空配置、最窄窗口和实际菜单／大纲跳转，正在由Mac owner取得。

首个1060×760完整原生窗口证据：运行时约72px、模型约133px，输入正文全宽，页脚布局符合方案；detail右上大纲／新建正确。但SwiftUI将sidebar `.navigation` 工具栏菜单合并到tracking separator右边（x270，sidebar240），不能以源码归属代表真实位置。root未采用“移至搜索行尾即可视为toolbar”的直接替代，要求继续原生侧栏工具栏定位，并委托advisor判断具体平台入口；暂不交付位置仍错的安装包。

侧栏位置决定：advisor核对Apple公开文档，`.navigation`只保证窗口leading而不保证列归属，sidebarToggle不是任意item placement。owner仅再用`.automatic`做一次实际判别；仍错位时采用侧栏自身独立顶部工具栏，原生标题与Menu，搜索另行，随列调整和隐藏。用户要求的是侧栏工具栏作用域，没有要求接管窗口标题栏；root不为单个筛选按钮引入NSToolbar／NSSplitView管理层或猜内部identifier重排，也不把搜索行尾冒称工具栏。采用的具体呈现已告知用户，实际整窗复验仍待owner返回。菜单直接列选项、选择下一轮运行时不改变历史、大纲／新建在右侧以及菜单设置入口已在合成完整窗口确认。

后续实测推翻需要侧栏自有内容工具栏的候选：`.automatic`在本机实际将筛选移至x150，位于sidebar240px tracking separator左侧；默认侧栏toggle在x107，右侧大纲／新建保持。因此最终保留公开SwiftUI系统工具栏placement，不增加内容工具栏或AppKit管理层，已向用户说明。实际拖拽缩到780×592后筛选仍在左区，大纲／新建x713／754，短名称页脚两菜单约220px，发送x741且无溢出。隐藏侧栏时筛选由系统收进overflow，展开恢复左区。大纲选择第二用户消息成功跳转并保留回到底部箭头；新建打开原生sheet，未执行真实创建。长名称与最后静态检查仍待owner收口。

长名称证据再次修正方案：Menu上的fixedSize使其intrinsic宽度突破label maxWidth，780窗内运行时约238px且未截断。root采用紧凑上限优先于自适应内容宽度，不为该布局引入手工测量或AppKit封装；实际截断、完整help／AX、发送／停止可达是决定标准。最终说明使用“紧凑直接菜单”，不再承诺每次恰好按内容宽度。owner继续修复并实测最大sidebar宽度／长名／停止状态，不以静态frame冒充有效约束。

Mac最终返回已采用并冻结：Views.swift／Transcript.swift，严格Swift warnings-as-errors（6.45s）和diff检查通过。完整1060×760及实际拖拽780×592窗口（sidebar339／detail441）已确认：自动位置筛选在左区，新建／大纲在右，完整88高编辑区域与单行页脚；Menu外层约束120／160使长名称真正截断、完整名称由菜单／help／AX保留，发送可见。运行时只改下一轮目标，历史内容不变；两菜单设置入口打开原生设置，大纲选用户项后跳转，新建原生sheet打开。完整面板生成态没有实际触发，不以布局推导冒充该状态GUI验收；IME仍未实测。所有合成观察副本正常退出清理，无真实资料访问。root开始最终整包构建与安装，Rust未变化，不重复此前合成6请求及workspace静态检查。

本次UI调整已交付：标准release整包构建通过（Swift warnings-as-errors，27.04s），正常退出旧应用后重新安装并打开0.1 beta.1。安装二进制、Rust动态库及manifest与本次构建一致，严格签名通过；未提交，dirty=true忠实反映当前工作树。PRD／Mac README／开发说明中的输入面板、大纲与工具栏归属已同步，局部文档链接及工作树／暂存diff检查通过。UI修改已完成；原Pi真实启动退出原因仍待用户新版重试，不以本轮界面交付替代诊断结论。

用户随后提醒自主提交授权。本任务此前未提交是收尾遗漏，不是新的审批要求；按用户已明确的持续迭代授权补齐本地提交，只包含本packet对应的Mac输入面板／工具栏、Pi错误原因传播和人工验收、相关权威说明。复用已完成的静态检查、构建与隔离验收，不因提交重复行为检查；不push或创建PR。UI代码与当前安装包一致，安装manifest保留本次提交前的真实构建状态，下次应用改动重建时更新关联。

2026-10-08 用户在新版继续Pi既有会话发送仍失败，授权查日志并修复；同时要求列表筛选／分组重启记忆。恢复此packet，不把前轮隔离通过当成真实原因已解决。pi_install_repair继续拥有新失败日志诊断、Rust执行修复和隔离验证；session_management继续拥有Mac浏览偏好与必要application配置契约，root维护长期文档、整合检查、构建安装及当前任务提交。真实凭据／正文不读取，真实提供商不调用。完成依据是新日志cause得到解释并在隔离路径修复、浏览偏好新实例／重启恢复、必要静态检查及整包安装；真实服务体验留给用户。

本轮新失败日志确认startup readiness RPC超时（总操作约4秒，响应窗口3秒），没有留存进一步stderr原因；这不同于前轮child exit，但旧timeout没有查询状态，不能据此断言当时子进程仍运行或排除其它真实原因。隔离4秒启动延迟能够检验短等待窗口。修复将首次state握手的单次响应等待扩大至15秒，普通发送／取消控制仍3秒，并补超时进程状态。真实调用不由开发方复现。

浏览偏好owner返回已采用：application/UniFFI具名preferences，schema7新增默认字段；grouping/sort/oldestFirst/runtimeID/project保存，初始未加载禁用菜单，旧list和旧保存回调不覆盖新选择，写失败恢复最后durable并记Problems。临时Swift人工脚本贯通AppStore→Transport→UniFFI，临时HOME下defaults、全枚举roundtrip、连续3次latest、旧list、重开恢复全部字段与groupLimit37、search不保存、atomic失败回滚通过。workspace fmt/check/clippy及application no-default检查、Swift warnings-as-errors通过。root复核字段与状态传播、脚本隔离边界后采用，不重复owner人工检查。

Pi最终证据已采用：state()三个readiness RPC用15秒静默响应窗口，ordinary RPC仍3秒。timeout查询区分alive／exit／状态查询失败；持续非匹配事件会重新等待，不承诺绝对总15秒。实际外部Pi1.0.2+合成配置延迟4秒完整6请求通过，延迟16秒在约15秒失败且带alive和实例入口上下文。新记录足以修复短等待边界，但不证明真实会话仅此问题；若重试仍失败从新诊断继续调查。

整合完成：release Rust＋当前UniFFI＋Swift warnings-as-errors整包构建通过（Swift29.91秒），签名通过。正常退出真实应用，重新安装／打开Applications中的0.1 beta.1；可执行文件、Rust库、manifest逐一SHA256与构建包一致。PRD、Mac README、开发说明同步，局部文档链接及diff检查通过。本次安装manifest忠实保留提交前508c7c4/dirty=true的构建状态；源码无后续变化。准备自主本地提交本轮19文件，不push/PR。浏览持久化与启动窗口修复已交付；真实调用最终反馈留用户。

2026-10-08 用户在510d9e0新版再次重试仍失败；当前恢复诊断，前轮只修复短等待边界，不认定真实发送已恢复。继续稳定Pi owner核对新operation_id/cause，禁止盲目再扩大timeout。Root检查SDK装配与readiness链路，保留真实来源与协议边界；有新根因时隔离复现、修复、静态检查、安装及自主提交。

本次重试根因已确认：UTC04:33:09，operation_id 18dc73b7cf1fc918-15fcc-2b，runtime_start_failed原始cause为“当前接入不支持注册直连 AI 提供商的 Pi 扩展”，exit1，约4068ms；不是timeout。pi_rpc.mjs将任何extension provider注册等同于绕过gateway，启动时一刀切拒绝。owner已用合成provider扩展完成6请求，但root尚未仅凭固定初始模型选择采用其“始终经gateway”断言；advisor核对namespace/运行时模型选择边界，owner补实际route与无direct请求判别。真实扩展内容不读取。

advisor判别已采用：Pi公开1.0.2 SDK允许运行中registerProvider/registerVirtualModel及pi.setModel，空credential store不防配置apiKey；prepareRequest为private，不覆盖它。主turn/compaction/summary/cache经公开ModelRuntime.streamSimple，virtualdirect resolve后递归同入口。删除普通注册禁令，准确保护适配器owned名称，实际派发核对注入gateway并委托原方法；不引入扩展sandbox。owner须验收普通注册/continuation成功且网关收到请求、选普通provider不会调用模拟上游并报明确错误、保留名称冲突具体诊断。

最终Pi实现已采用并冻结：普通provider注册允许；拦截保留provider/virtual名称覆盖，explicit冲突变量；streamSimple同步守卫放行ownedauto虚拟模型和snapshot catalog匹配的gateway物理模型（expected必须存在，核对api/baseUrl）。自有virtual注册带内部标记适应SDKreload，不将该标记当恶意扩展隔离证明。实际SDK派发继续委托原方法。现有manual-pi-native-loop.py临时PI_HOME添加普通fixture-direct扩展，6请求完整gateway loop通过；临时真实SDK脚本在set_model直连后prompt产生明确guard错误；reserved provider/auto冲突check通过。Node syntax、Python compile及diff检查通过。Root复核实际source和directguard脚本后采用证据，Rust/Swift源码未变不重跑无关静态检查。真实扩展／凭据／正文未读取，真实上游未调用。开始标准构建安装。

本轮整合交付：标准release构建完成，Swift warnings-as-errors 12.47秒；正常退出旧App、安装并重新打开Applications中的0.1 beta.1。安装二进制、Rust库、manifest和两个Pi helper与构建一致，两helper另外与源码逐字节一致，严格签名通过。Runtime README／开发说明同步，局部文档链接与diff检查通过。manifest忠实记录构建时510d9e0/dirty=true，不伪改构建来源。自主提交本轮6文件，不push/PR；当前故障原因已修正并隔离验证，不代替真实会话最终验收。
