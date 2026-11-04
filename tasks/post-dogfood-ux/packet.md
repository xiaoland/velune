# Dogfood 后的输入、执行与导航改进

2026-11-04，用户在工作区收尾期间追加12项。前一批交付继续完成、关键验收并提交，新增工作在同一任务链继续，不遗忘网络故障的未决归因。用户要求加快，子代理优先 GPT-6.1 Sol / xhigh；后续明确取消fast mode，并强调使用svc-workflow的design method。禁止抢鼠标、CUA、前台激活、Windows UI、真实会话／凭据及日常重装。仅分层关键路径验证，复用既有证据。

## 新增范围

1. 系统通知打开会话用新Tab，已打开则聚焦，不替换当前Tab。
2. composer状态详情显示当前所选提供商额度，核对Kimi For Coding与原生／managed来源。
3. 会话打开2–3秒的关键慢路径。
4. 429之后的重试不得意外从Pi切成Codex；用户随后修正记忆，可能手动发过retry，但未切换运行时。区分原生自动重试与应用下一轮选择。
5. 在途执行期间允许保存提供商及运行时配置；以执行快照／新配置分代隔离，不按字段枚举编辑许可。
6. 核实并接通Codex166整项目选择：项目身份与全部运行根同时提交，cwd默认首根；保留单目录／无工作区。
7. 消息搜索高亮并按需展开工作过程。
8. ⌘Return“插入本轮”的菜单／快捷键准入。
9. 工作过程footer不显示创建时间；消息时间随copy／fork／edit组成hover bar一起显隐。
10. 跨group聚焦保留正文和附件，执行配置跟随目标（用户已明确确认）。
11. composer中间区域宽度按可用空间比例配置。

## 已确认的输入规则

内容身份只负责正文、持久附件；目标上下文负责运行时／模型／推理／权限选择。`InputID`随手头输入延续；`TargetKey = conversation(ownerId, conversationId) | newConversation`恢复执行配置。同group导航换Tab时取目标的内容关联和配置；跨group仅移焦点时保留内容ID，切换目标配置；布局整理不换正文／附件，目标未变时也不换配置。新建仍只有全局一个内容关联与一套选择，多新建Tab的cwd等创建目标继续各自保存。

内容与配置各自持久化、跨设备同步与修订；执行owner用于消歧，不能按查看deviceId隔离。发送捕获当时内容ID与目标配置快照，迟到回包不能消费后来编辑。旧组合记录必须显式迁移，不能靠UI在聚焦时覆盖同一份输入的选择。

## 所有权与依赖

- mac_navigation_and_transcript：通知路由、搜索／展开／高亮、message hover bar；Workspace只改通知路径。
- composer_controls：Composer／Context局部控制、额度调查、比例宽度与离线迁移、main菜单／快捷键；不改状态存储。
- runtime_lifecycle_and_latency：429身份、在途配置快照、打开性能；前批fork完成后接手共享生命周期文件。
- 主Agent：输入状态方案与契约整合、project事实核对、构建调度、文档和提交。

前三个新增owner先只读调查，前批工作区提交后统一放行，避免混入半成品。任务过程与证据目录 `/tmp/velune-next-ux`，完成时归档到target/verification。

## 验收原则

以可观察缺陷构建最小合成场景；源码／契约检查先行，实际运行时只验改变的生命周期，Mac隐藏原生检查只验新的关键交互和视觉。跨平台只在共享契约变化后构建，Windows永不启动UI。网络故障仍见[前批取证](../workspace-delivery-and-network/packet.md)，不能把本批体验修复宣称为断网根因修复。

## 已获得的新证据

- 429基线：实际Pi1.4在合成上游429后原生自动重试仍为Pi；用户后来澄清可能发过retry。源码找到既有目标尚在加载时从旧store.nextTurn播种运行时的具体路径，输入／目标拆分已移除该路径，已由最终隐藏原生回归确认。
- 在途保存：实际Pi429等待期间保存provider endpoint/API key与runtime成功；旧retry仍旧endpoint/Key，下一轮使用新值。运行中将binary改为不存在的合成路径，当前轮仍完成，下一次准备明确失败且不产生上游请求。`runtime/config_retry.py --save`及`runtime/report.md`记录，未真实联网。
- 网关确为每执行会话一份Runner；stable provider的SchedulingState跨Runner／配置快照共享。已有额度快照与采集取消通道按代际隔离，旧凭据清理及epoch退休等无活跃引用再执行。删除／显式logout保持独立保护。Mac／Windows保存按钮只按本身保存或登录状态禁用，没有会话运行状态门禁。
- 打开慢的因果复现：合成Pi扩展使native catalog初始化约2.4–2.6秒，历史open中位约0.12–0.14秒；原先composer事件使catalog抢在open前入队。新增guard把模型目录加载安排在目标打开完成后，不扩大Transport并发。
- 多根项目：实际Codex166的projectId-only请求只见cwd；projectId+runtimeWorkspaceRoots请求见全部根且获得对应写入范围。实际application新入口的native／managed、关闭重开保留、普通cwd不扩大、跨runtime key拒绝均通过。
- 离线core额度检查确认Kimi未知／加载状态仍有条目、route同provider去重、切换provider换条目；UI现在观察当前model并重新解析目标，不持有旧context。没有真实账户额度联调。
- 内容／目标schema2显式迁移已用合成数据验证：正文附件保留、逐context复制选择、孤立旧数据备份、pending原baseline保留、预览不写、锁与幂等。实际共享API双client内容／目标独立CAS已通过；4项Rust窄测试通过，含旧target修订拒绝且正文保持。Mac最终规则与真实双client检查已通过。
- Mac原生新增navigation-only、composer-controls、content-target三个窄入口；仅执行这些新增路径，不重复之前全套；最终均已通过。Android assemble／lint、Windows隔离release及publish、iOS最后Swift增量构建均通过；Mac完成SVC完整构建与最后窄增量。最后CAS界面小补的各端验证边界见下文。

## 收尾设计复核

按内容延续、目标归属、执行快照三个不同生命周期追踪换Tab、移焦点、创建迟到回包和重试。相比继续共享整份输入或全部per-session，内容／目标拆分分别保留用户确认的两种直觉。集成读取调用链发现侧聊不在共享ContextKey目录却被误导向目标保存接口，已恢复其专属store的临时选择路径，并补入同一隐藏检查中的侧聊模型／运行时切换、能力拒绝与关闭重开隔离。这属于拆分边界的必要修复，不能只凭主会话测试通过交付。

## 当前集成证据

- `clippy-final-2.log`：workspace／all-targets／all-features release严格Clippy通过；只修两处初始化写法，复用已有行为证据。
- `ios-build-2.log`：最后源码Swift增量构建通过；用临时project副本去除强制native构建phase，实际源码和已生成原生库均来自原目录，仓库project未改。
- `windows-build-1.log`：本批Rust、绑定、C#、publish、资源打包通过，未启动UI；`windows-cleanup.log`确认先解除junction再删除隔离worktree，共享缓存保留。
- Mac build1修正两处接口编译错误；build2的cargo生成器停在文件metadata stat（采样见mac-bindgen-sample.txt），已终止自己的构建；build3由SVC调用同一脚本，临时wrapper仅复用已编译的通用bindgen可执行文件，仍读取本批新dylib生成绑定。

- Mac SVC build3完成：`6aa21db0-6bd5-4fb6-b00c-6c4fc6eb251a`。navigation-only通过，通知模型路由不替换现有Tab，重复／跨窗复用；搜索展开／高亮／关闭清理；hover时间、工作footer无时间；catalog打开准入。原生PNG已目视，无真实系统通知点击。
- controls入口通过快捷键、比例、选择额度跟随断言；visual-only宽1100／窄560截图已目视居中且不裁切，未重跑旧功能检查。额度单张离屏图缺底色，下一次Swift增量仅补该视觉。
- 追加设计复核发现目标CAS冲突缺少可操作恢复入口：保持后台原基线，用户明确同值或新值重选才采用当前整套配置、以最近观察修订提交；四端补同一闭环，不新增按钮／自动合并。

## 最后补丁的验收范围

四端构建已通过后，CAS恢复闭环仅修改各端已有选择保存路径：Mac最终Swift增量后用双client检查实际冲突及同值重选；iOS／Android按已有实际类型做Swift parse／局部调用链核对，Windows只有3处局部改动并核对同值Click路径，均未重跑原生核心／全套UI。此范围不被描述为四端最终源码重新完整构建。Rust最后仅对项目创建4处排版执行rustfmt，复用严格Clippy与实际运行时证据。

Mac最终Swift增量复用SVC build3生成的原生库、绑定和资源，只执行同一脚本里的Swift编译／替换本地产物／签名校验步骤，没有安装或启动日常应用。首次增量期间owner补充worker层禁止后台重基线，故冻结后再补一次Swift增量，其他已通过路径不重验。

## 使用边界

- 源码修复了尚未加载目标时沿用旧运行时选择的可复现场景；未读取用户那次真实会话，不能将它宣称为该次故障唯一已证实的原因。已经写入的旧会话选择不会凭推测重置，真实配置仍须显式迁移；如旧记录已选Codex，应明确选回Pi，而不是替用户猜测意图。
- Kimi额度通过合成账户状态和通用UI路径验收，未调用真实账户。打开延迟改的是catalog排队顺序，不承诺所有历史读取固定低于某个耗时。
- 完整宿主之间共同composer保存端的部署仍沿用前批待决边界；本批分别同步内容与目标，不按查看deviceId隔离。
- 网络故障保持前批取证结论与未决归因，不将本批提交标为网络修复。

- 最终Mac Swift增量2及签名验证通过，日志`mac-swift-final-2.log`；不再修改产品源码。controls最终入口通过，额度PNG已由owner与主Agent实际查看，Kimi 5h／7d信息可读且无裁切；账户为合成观测，时间不代表真实用量。
- `fmt-final-verified.log`通过；Markdown相对链接267个无问题；`git diff --check`通过。
- 原生retry检查首次只在临时fixture准备失败（Codex配置漏nodeBinary，尚未启动Mac），仅修验证脚本后继续相同入口，不能记为产品失败或放宽准入。

- 最后的隐藏入口经LLDB定位到验证前置缺失：侧聊选native模型前未走真实prepare，两个DeviceStore未start加载配置，原生Store未start轮询。只补正常生命周期及独立最小runtime fixture；未放宽产品模型准入。DeviceStore在`.prohibited`检查进程不建立系统通知投递器，避免合成完成提醒出现在桌面，正常应用策略不变。`mac-swift-lifecycle-final.log`编译／签名已通过。
- 同值重选沿core原有Some结果，不另加拒绝后的fallback。CAS检查的旧pending改为显式revision 2以避免自动poll时序造成未真正触发冲突；若先前产物已通过冲突断言，则此单行fixture改动按parse／类型核对覆盖，不再为它重编完整模块。

## 最终结果

- `composer-native-regression.log`：全部通过。同group换Tab恢复内容和目标，跨group只保留正文／附件；布局连续；侧聊拥有独立临时配置并拒绝无能力runtime；失败恢复不会覆盖后续编辑；真实双client内容／附件／配置独立同步；CAS冲突保留本地、后台不重基线，同值显式选择可恢复且正文revision不变。
- 实际Codex原生历史 → 带1.8秒catalog延迟的Pi历史 → 已存在429失败 → 输入retry：只有Pi发出后续请求并完成，目标选择仍为Pi；回环共捕获一次初始429和一次Pi retry，无Codex发送。最终合成目录`/tmp/velune-composer-native-hfcg9bvn`，不含真实会话／凭据。
- 最后两次fixture故障均经LLDB定位：未加载目录及用空白恢复API模拟改字；保持原产品准入与“new edit”精确断言，修正真实生命周期／模拟动作后通过。此过程的SIGTRAP来自隐藏检查precondition，不是日常应用崩溃证据。
- 最终本地产物：SVC build3 + `mac-swift-input-fixture-final.log`，签名验证通过。通知／搜索／hover、controls与宽窄视觉复用各自已通过结果，不因fixture更正重复。
- 证据归档至`target/verification/post-dogfood-20261104`（忽略、不入git）；真实数据未迁移，日常应用未重装，Windows隔离环境已清理。网络归因仍开放。
