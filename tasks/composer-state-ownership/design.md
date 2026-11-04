# 实现与验证安排

## 2026-11-03 dogfood 修订实施

用户已授权按修订方案落地，聚焦新建页沿用当前输入，新建命令才恢复全局新建项。本轮已按以下责任边界实现：

1. application-api 增加稳定 InputID、全局 ContextKey、完整输入内容与修订操作；application 将目录放在 ClientState 之外，由已有串行 ApplicationService 持有并原子持久化。上下文包含执行来源以消歧，不包含查看设备或窗口。
2. 附件使用持久内容身份，可经已有已认证业务通道上传／读取；不复用 view 私有短期引用作为同步身份。跨执行主机发送时通过发送端已有附件入口交付实际文件。
3. Mac 的编辑器继续负责原生文本／撤销；窗口负责导航上下文、当前 InputID 和发送目标。显式导航保存／恢复关联，聚焦只改目标，布局延续当前工作。新建命令解析唯一 newConversation，首发得到原生身份后交接输入；发送失败不重新创建。
4. 其他客户端接入相同完整记录，浏览／布局保持本地。单用户依序接续；逐输入 CAS 防旧缓存，未知提交结果沿用远控查询，未完成编辑不盲重试覆盖。
5. 用合成旧文件验证一次性显式迁移，再做共享核心不变量、两个独立客户端／view 的真实协议组合、Mac 后台原生窗口动作与进程恢复。仅检验受影响关键路径；复用未受影响的既有运行时证据。

全局目录的保存主机已向用户询问，建议当前 Mac；尚无答复。独立保存端 facade 已实现并验证，但未配置真实设备，完整执行宿主的设置接线仍未交付。当前部署与迁移边界见 [packet](packet.md)。

## 本轮分层证据

证据均为合成 HOME／VELUNE_HOME／运行时，不访问真实账户、凭据或会话。只补受影响关键路径，不重跑侧聊、频率限制或完整导航矩阵。

| 层次 | 观察与适用结论 |
| --- | --- |
| 共享核心不变量 | `/tmp/velune-composer-core-tests-final-5.log`：3 项通过，覆盖目录／附件跨 view 与冷启、输入和关联的旧 CAS 拒绝、借用 X 创建不消费 N、同 ID 不同执行来源、重复创建交接不消费后来恢复的新建项、删除后迟到交接不复活关联。严格全特性全 targets clippy 通过，日志 `/tmp/velune-composer-clippy-final-2.log`。 |
| 实际协议组合 | `/tmp/velune-composer-state/protocol-final.log`：两个已认证 loopback remote view 共享内容与修订，旧关联被拒绝；共享创建记录带来源且幂等；独立保存／执行 owner 下，持久图片经真实执行 view 上传后到达 ACP prompt；释放 view 不删除图片。不是只测序列化。 |
| Mac 导航与恢复 | 复用本轮 `native-write.log` 的实际窗口动作：同组切 Tab、跨组聚焦、split／close 连续、明确新建及借用输入身份。`native-sync-final.log` 验完整内容同步、失效附件局部恢复与强制重试；`native-reopen-final.log` 验新进程恢复同 InputID、正文、附件及执行选择。均在 `/tmp/velune-composer-state`。 |
| Mac 创建生命周期 | `native-lifecycle-final.log` 验无需正文编辑也保存 cwd、内容值不变也更新提交资格、另一新建编辑器接续原生身份并保留同输入。原生身份为合成占位，仅证明身份切换，不声称对应运行时历史加载成功。build13 的 `native-targets-final.log` 验多个新建 Tab 共用同 InputID、各自 cwd，窗口 JSON 往返及后台 Tab 延迟恢复保持目标与输入。 |
| 显式迁移 | `migration-final.log` 与 `migration-native-final.log`：dry-run、运行时锁、Swift UUID 字典和模型编码、唯一新建、附件缺失留引用、原文件不变、拒绝覆盖；最新目录被实际 application 接受，关闭重开保持完整身份与内容。未对真实数据执行。 |
| 平台构建 | 最终 Mac SVC build13 `6e3795ca-1fcd-4dae-aa6f-73aadfbbab56` 通过，日志 `/tmp/velune-composer-state-build-13.log`。iOS 模拟器 `/tmp/velune-composer-state-ios-build-6.log` 通过；Android assemble／lint `/tmp/velune-composer-state-android-build-5.log` 通过；Windows 隔离目标机完整构建 `/tmp/velune-composer-state-windows-build-4.log` 通过。最后改动只涉及隐藏检查数据，其他端构建不重复。 |

新增 Mac 目标检查最初失败于预览 fixture：预览新建并非待创建状态，且 Store 自动载入 sample-0，与后台 Tab 延迟恢复的前提冲突。最终改用合成 HOME 的真实 application，保留正常延迟恢复顺序后通过；未为检查改动生产导航规则。

Windows 隐藏原生 harness 在无 Show／Activate 的情况下仍触发前台保护，检查失败后退出，结果保存在 `windows-hidden-native-result.json`；不算主题、原生交互或视觉通过。已停止运行并移除本任务的 scheduled tasks／worktree／临时脚本，缓存 junction 用无 `/S` 的 `rmdir` 解除，日常缓存保留；日志 `windows-cleanup.log`。Mac 隐藏检查同样不算视觉检查，本轮未改 composer 的视觉布局。

以下为此前实现与验证的历史记录，不能代替上述本轮证据。

## 先前实现（历史）

用户已批准产品方案并要求实现本轮全部修复；本文件记录技术选择，不增加提交或日常重装授权。

## 输入与目标

`ComposerDrafts` 属于会话窗口，管理输入的稳定 UUID、会话／共享关联和被挤出的未完成输入。`ComposerModel` 保留原生编辑行为，生产路径使用每份输入独立的文本／附件／意图／撤销，不再由视图随目标替换正文。`ComposerContext` 继续配对当前输入与操作目标，核心解析选项；共享焦点切换改变目标，布局转换移动关联。窗口恢复保存图中实际引用的输入；会话主草稿与选择继续写入现有设备存储接口。`DeviceStore` 同步同一会话在各窗口中的完整编辑状态；初始化、恢复与消费输入只发布完整语义结果，接收方不回写回声。标签跨窗口移动复用原编辑模型，保留原生撤销；独立共享输入不随目标标签离开原工作区。

布局转换不创建文本副本，目的处另一份输入进入保留列表。草稿菜单只在共用输入区或有可恢复内容时出现。清空／失败恢复按捕获的输入对象与修订处理。删除会话清理仍归属于它的主项与保留项，已经转入共享／其他会话的输入不受旧来源删除影响。

本地草稿编辑资格与核心提交准入分开。外部控制仍禁止发送／排队等运行操作，但不禁止原生文本、粘贴和本地附件编辑；真正只读的原生子会话仍只读。输入框最低一行；drawer 下限由一行与既有操作行／边距构成，默认高度仍为 220pt，调整后不因状态改变重算。

## 侧聊

侧聊的标题由核心投影固定为来源名称，Mac 标签消费相同标题。`SendSideChat` 接收目标运行时、模型和 ComposedTurn；下一轮选择器使用共用解析器，不固定在打开时的 NextTurn。只列出实际支持临时侧聊的运行时。

不改变执行配置时沿用当前原生侧聊。配置改变时保留来源参考与已有侧聊问答，创建新的临时上下文：Codex ephemeral thread、Claude 无会话持久化的 side question、Pi in-memory session。历史作为引用而非新指令，不重放工具或审批；旧侧聊轮次前缀留在同一公开侧聊身份下。创建失败恢复原侧聊投影；派发失败不自动重发。关闭回收原生副本与应用拥有的临时上下文文件。

## 频率限制

按 Codex 0.159.3 的官方 ErrorNotification 契约分类 `rateLimitExceeded` 以及传输错误变体的 `httpStatusCode=429`，不通过模糊错误文案推断账户额度。新增 RuntimeFailureKind.rateLimited，复用已有非阻塞、有界的空输入恢复；原生 `willRetry` 优先，停止／耗尽语义不变。Pi 等已有原生自动重试仍由原生执行，未知的恢复契约不伪造。

来源：[官方错误 schema](https://raw.githubusercontent.com/openai/codex/rust-v0.159.3/codex-rs/app-server-protocol/schema/json/v2/ErrorNotification.json)、[错误语义](https://raw.githubusercontent.com/openai/codex/rust-v0.159.3/codex-rs/protocol/src/error.rs)。

## 分层证据与结论

只验与当前决策相关的关键路径。最后一次仅修改 Mac 的跨窗口同步和标签携带模型，因此只补局部不变量、Mac 编译和启动检查；前面的侧聊、频率限制、布局及进程恢复证据继续适用，未重跑完整矩阵。收尾曾多重复一次会话切换，用户指出后停止扩大验收。

| 层次 | 证据与所支持的结论 |
| --- | --- |
| 局部规则 | agent-runtime 的 28 项测试通过，包含语义频率限制、HTTP429、有界等待、停止、原生重试优先和账户用量排除。`/tmp/velune-composer-runtime-tests.log`；严格 clippy 与 cargo fmt 检查通过。 |
| Mac 状态不变量 | 最新构建的 `--check-composer-sharing` 通过：同一会话文字与明确选择同步、消费原子清空、移入共享输入后来源不留副本。`/tmp/velune-composer-ownership-ui/sharing-check.log`。 |
| 实际 application 组合路径 | Codex 0.159.3、实际 application、合成 HOME 和回环 Responses 服务完成侧聊三轮：沿用配置、换模型、换运行时实例。实际请求模型正确，引用及前轮问答保留，公开侧聊身份不变，原会话文件逐字节不变，临时侧聊无持久 rollout。`/tmp/velune-composer-ownership-ui/side-switch-e2e.log`。 |
| Mac 原生界面 | 隔离预览实际操作并截图：单栏 A/B 草稿切换；锁定仍能编辑、换行、添加附件；分屏焦点／目标替换延续输入；关闭分屏延续 X 并可恢复原 Y；草稿往返、撤销与重做；隐藏 drawer 后修改运行时；一行最小尺寸及紧凑附件详情。实际 application 的原生侧聊标签显示来源标题，运行时和模型命令可选择，关闭侧聊后来源保留。 |
| 进程恢复 | build8 隔离预览正常退出后以新进程重开，恢复目标 B、当前 X、保留 Y、附件、明确运行时和最小高度。预览 `start` 曾忽略恢复目标，已修正其合成数据启动入口后验证。 |
| 构建兼容 | 最终 Mac SVC build9 `2f5914b4-14ea-471f-aa39-79336829cb5a` 通过；iOS 模拟器构建通过；Android assembleDebug 与 lintDebug 通过；Windows 目标机隔离 worktree 的 Rust DLL、C# 绑定及 dotnet publish 通过。后三者复用未受最后 Mac 改动影响的构建结果。 |

Mac 最终构建日志为 `/tmp/velune-composer-ownership-build-9.log`；其他端分别为 `/tmp/velune-composer-ios-build.log`、`/tmp/velune-composer-android-build.log`、`/tmp/velune-composer-windows-build-2.log`。独立预览及侧聊检查副本均已正常退出，本任务 Windows 临时 worktree 已移除，未改动日常安装，未提交。文档相对路径／锚点检查覆盖 10 份文件、169 个链接，0 错误；`git diff --check` 通过。

## 验证边界

- CUA 坐标 drag 返回 `noWindowsAvailable`，原生 AX、按键与截图正常；最小高度通过独立预览标志设定后目视检查，未声称实际拖拽手势通过。跨窗口状态通过局部检查验证，未声称物理拖拽和撤销跨窗口完成原生 UI 验收。
- 侧聊改配置的完整执行证据覆盖实际 Codex 的两个实例与多个模型；Claude／Pi 的适配路径已实现并编译，未声称跨运行时家族问答通过。
- Windows 仅完成目标机原生构建，没有通过 Windows App 或远程桌面替代 UI 验收；本次 Windows 锁定编辑修复尚缺目标机原生 UI 证据。Mac 的状态归属、侧聊选择与一行 drawer 尚未移植至其他客户端，差异见 Windows PARITY。
- 真实账户、真实凭据、真实会话与外部模型请求均不在本轮验证范围。产品方案已获用户批准；上述证据不代替用户对实际体验的验收。
