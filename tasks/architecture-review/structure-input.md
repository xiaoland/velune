# Mac 输入内容所有权收敛

`ComposerModel` 现只拥有一个 `InputID` 的正文、附件、编辑修订和原生撤销历史。导航由 `ComposerDrafts` 选择模型，持久正文只由 `ComposerInputClient` 保存；侧聊的临时模型由 Tab 持有。输入内容和目标配置的既有产品规则、共享目录及窗口 schema 均不变。

## 已核实的旧路径

全部生产输入视图，包括 pane、停靠／浮动输入区、会话 sheet 和发送 sheet，都通过 `window.composer(for:)` 取得 `ComposerDrafts` 创建的独立输入模型。侧聊也走了这条路径。旧 `followsConversation=true` 只剩 `ConversationTab` 初始占位模型与旧检查显式构造，但其完整的按会话切正文、附件和撤销历史机制仍在模型内；`DeviceStore` 也继续读取旧正文并保留另一套延迟写入队列。

这使一个输入模型有两种内容归属规则，并让临时侧聊正文加入共享输入保存。侧聊虽然没有普通会话关联，正文仍可能成为共享目录中的孤立输入，随后出现在“保留的输入”中。

## 删除与责任

- 删除 `followsConversation`、`show(conversationID:)`、`reloadDraft`，以及按会话索引的附件、修订和撤销字典。模型切换目标只更新编辑能力与补全，不加载另一份正文。
- 删除 `DeviceStore` 的 `composerDrafts`、`composerEditingStates`、`unsavedDrafts`、正文广播与读写／移动／退出补写通道；删除 `ConversationStore` 的正文桥接、侧聊临时文字副本，以及 Mac `Transport` 的旧草稿入口。其它平台仍使用的核心兼容 API 未在本组修改。
- 发送、排队和 steer 捕获原输入模型与清空后的单一编辑修订。迟到失败只恢复该模型；换 Tab 不改变捕获身份，后续编辑使旧恢复失效。
- 共享目录投影更新模型时推进本地修订并通知视图，但不发出“本地编辑”保存信号；因此删除 `ComposerDrafts` 用来阻止同步回声的窗口级 `receiving` 状态。
- 跨窗采用已有 `InputID` 时，目的窗口以移入模型替换同 ID 的旧实例和订阅，并通知依赖方重新取值。共享 client 的当前内容优先；只有内容一致时保留移入模型原有撤销历史，旧移入正文不会覆盖较新内容。
- 跨窗拖放和新窗口 handoff 会把原编辑模型交给目标窗口，并在源 registry 留下同 `InputID` 的新本地模型、重接源窗口其它 Tab 的引用。源模型保留共享 client 的未同步内容，撤销独立重新开始；因此两个窗口不会共同持有一个只容纳单个原生 textView 的编辑模型。普通关闭和同窗布局移动不执行这次置换。隐藏检查使用实际 Tab drop 并保留源发送 sheet，断言两边对象、撤销与内容同步的关系；运行结果待统一构建验收。
- 窗口入口为侧聊返回无持久 owner，导航、布局搬移、保留项选择和跨窗采用均在进入持久 registry 之前判断来源与目的。侧聊 sheet／拆分切换使用同一 Tab 模型，关闭后重开使用新身份；不写共享 client，也不进入窗口输入快照。侧聊可以复制文字，但不能借用持久 `InputID`。
- 原生输入命令绑定明确的目标 Store；模型或目标变化时重新绑定，已删除的内容模式不再决定快捷键目标。

`app/mac/README.md` 已更新为现行职责。正式契约仍由[输入身份与同步](../../docs/design/composer-state.md)维护，任务本身不建立另一份产品规范。

## 验证与入口

静态检查已通过：本组改动 Swift 文件的 `swiftc -frontend -parse`、`git diff --check`、Mac README 16 个相对链接／标题锚点。旧正文缓冲、`followsConversation`、按会话模型切换与重载的源码引用已清除。

旧 `--check-composer-state` 依赖“换 Tab 仍保留独立共享输入”等已失效规则，与其 recovery-write/read 入口一并退役。旧参数显式返回 exit 2 并给出现行入口，历史任务包里的旧证据保持其历史含义。有效的 PNG／Finder 文件粘贴检查已迁到隐藏 controls 检查，不读取或替换用户剪贴板。

root 已使用本轮完成签名并隔离复制的 VeluneStructure.app 执行下列三项隐藏检查，均 exit 0。content-target 共 8 条 PASS，controls 共 6 条 PASS，new-tab 共 1 条 PASS。日志为本轮证据目录下的 native-content-target.log、native-controls.log 和 native-new-tab.log。

| 入口 | 本轮关键反例 | 隔离要求 |
| --- | --- | --- |
| `--check-composer-ownership --composer-check-content-target` | A 清空提交后切 B，迟到失败只回原 InputID；A 后续编辑优先。跨 group 换目标和整理布局后，正文、附件与原生 undo 延续。同 ID 移入时统一目的窗口模型，较新共享内容优先。侧聊 sheet／拆分、关闭重开、复制正文与尝试借用持久输入均不污染共享目录。既有双 client CAS 和附件检查继续执行。 | 设置全新的 `VELUNE_COMPOSER_CHECK_HOME=/tmp/...`；自行建立离线合成 Pi 配置，无需真实运行时或服务。不设置可选 `VELUNE_COMPOSER_NATIVE_HOME` 及其 CODEX／PI 身份。 |
| `--check-composer-controls` | 保留原生键盘／输入法检查，并接收从退役入口移入的 PNG 与 Finder 文件粘贴检查。 | 可设置 `VELUNE_CHECK_OUTPUT=/tmp/...`；命名 pasteboard 与合成文件。 |
| `--check-new-tab` | 复用现有首次提交、创建前后失败、全局新建输入消费和原生身份交接反例；清空／恢复改用模型修订。 | 按统一验收使用临时 HOME。 |

没有修改提交准入政策或 Rust 契约，没有执行真实迁移、安装、退出日常应用、CUA 或提交。真实 HOME 的迁移与部署仍遵守[总任务边界](packet.md)。

旧版本可能已将侧聊文字写成共享目录的孤立输入。孤立输入也可能是用户有意保留的普通输入，缺少可靠来源标记，不能统一删除。本轮阻止新的侧聊写入，未读取或清理真实目录中的历史孤立内容。
