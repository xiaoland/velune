# 消息呈现与工作区复核

本分支核对了消息／窗口相关任务包、当前 PRD 与真实调用链。发现并修复了执行设备、Artifact 对象、异步导航与通知呈现四类身份／生命周期缺口，保留既有呈现模型，没有增加核心同步机制。

## 已交付能力盘点

| 能力 | 当前实现与复核结论 |
| --- | --- |
| Mermaid Artifact | `ArtifactDocument` 保存来源与内容摘要，`ArtifactTab` 独立于运行时；sheet、Tab、分屏、跨窗共用编辑区承载。打包 WKWebView 本地渲染，CSP／导航策略阻止外部资源与图内跳转。跨窗重复查看缺口已在本次修复。 |
| 消息来源与最终回复 | 核心划分 turn／work／reply；Mac 仅消费投影。client 来源右侧展示，不增加用户大纲；失败／停止／未知终态不冒充成功折叠。既有脏改正在增加逐 turn fork 资格，本分支未覆盖其实现。 |
| 消息操作行 | `MessageMetadata` 统一时间格式，`TranscriptRowLayout` 在 hover 行放时间／copy／edit／fork；work footer 不伪造独立时间。当前源码符合后续 S78，早包中“工作过程底部显示时间”已被取代。 |
| 搜索 | `TranscriptSearch` 共用匹配／高亮规则；搜索期间读取 deferred 文本，选中结果时展开对应工作过程与 disclosure；关闭搜索清高亮。没有把渲染或 DOM 结构当业务搜索规则。 |
| 队列与全局进度 | 会话队列在详情、跨会话管理在窗口 toolbar；等待控制权和暂停原因独立。统一调度仍是当前设计，旧包的“未接 Codex 原生 queue 存储”是明确能力选择，不是遗漏后续实现。提交占位／回显接续由 composer owner 另行复核。 |
| 原生标题与附件打开 | 名称来自核心原生／生成优先级，Tab 不另存标题覆盖；附件普通 URL 交给系统打开。命名实际运行时边界由 runtime owner 复核，本分支不重复改写。 |
| 窗口恢复与默认值 | SceneStorage 与 journal 复用同一 snapshot；后台 Tab 保留会话与新建目标，独立窗口使用自己的内置 composer 默认。kind 已过滤，device 缺口已修复。默认工作区读取最近同类恢复点符合当前 PRD，未按早期设计撤回。 |
| 通知导航与 Tab 行为 | 通知在同一 `DeviceStore` 的窗口间复用已有 Tab，否则新开并保留当前编辑；零窗口时经已有 scene 入口开窗。抑制通知改看当前 key 内容，Artifact／设置不沿用旧会话。已知目标打开失败的身份／输入回滚与离表链接迟到结果缺口在本次修复。 |

## 本次修复及原因

1. **执行设备是窗口呈现的身份，不能只在首开入口携带。** `ConversationWindow` 用 `WindowSeed.device` 选择 `DeviceStore`，但 `hand`／`openInNewWindow` 丢掉了它；远端 Tab 的 store 与新窗口 application 因而指向不同 core。另因 remote transport 共用本机 state directory，journal 仅过滤 windowKind 时，本机默认回退可取到更新的远端快照。现在 seed 保留 device，`SavedWindowLayout.windowDevice` 与 journal／SceneStorage 同时过滤，列表和 Tab 拖放在跨 core 时保留原对象并拒绝接入。
2. **Artifact 内容身份需要跨承载查找。** 原 `openArtifact` 只查当前窗口；移出后再次查看会制造新对象，源码／图表选择也被重置。现在在同一 `DeviceStore` 的窗口中查找既有 sheet／Tab，选择已有呈现并保留对象状态；相同消息 ID 在另一执行设备上仍是独立来源。
3. **导航意图、加载中的目标和已打开内容有不同生命周期。** composer owner 已补 `selectConversation` 失败后恢复原 `isDraft` 与 composer 关联，本分支独立核对其真实窗口调用链。继续追踪发现离表链接 lookup 不受该操作 generation 保护，迟到成功会覆盖用户后选的目标；同链接前后两次请求也不能仅按会话 ID 判断。现在 lookup 有独立请求身份，后续导航、新建、关闭或 reset 会作废旧查找；当前查找才可发布结果。有更新 deferred 目标时，加载结束不再先安排旧目标模型目录。
4. **应用级通知意图不能依赖会话窗口存活，实际可见性也不同于上次聚焦。** 原通知在所有会话窗口关闭后找不到目标模型，激活应用后直接丢弃会话 ID；`isShowingConversation` 又只看最后聚焦 store，读取 Artifact／设置时仍会误抑制旧会话通知。现在通知和定时任务结果共用 `DeviceStore.showConversation`，复用已有 `OpenWindowAction`／`WindowSeed`，零窗口时请求 scene，已关闭但仍被 scene 持有的模型不参与路由。通知只在当前 key 窗口实际呈现该会话时抑制。

## 证据与边界

- `swiftc -frontend -parse` 已覆盖本次 10 个 Swift 文件，`git diff --check` 通过。
- 统一完整签名产物 `target/verification/architecture-review-20261105/VeluneReview.app` 已运行 `--experience-navigation-only`，exit 0，[日志](../../target/verification/architecture-review-20261105/native-navigation.log)共 9 条 PASS。新增 `WorkspaceContinuityChecks` 覆盖远端 seed、Artifact 交接与重复查看、跨设备 Tab／列表拖放、本机／远端精确及 latest 恢复、legacy 仅凭明确 scene UUID 恢复；当前搜索展开／高亮和 hover 行也通过。未创建可见窗口或注入输入。
- 同一窄入口的 `ConversationStore` 检查使用确定失败的隔离 transport，覆盖原会话输入、未首发身份与 deferred 导航恢复；在真实 lookup 完成边界注入旧成功结果，检查新目标及同链接新请求均保留。该检查未启动运行时或核心网络连接。
- 通知检查只注入开窗回调并传入隐藏 NSWindow 作为 key 观察值，覆盖零窗口、已关闭模型、Artifact 和设置内容，不请求系统通知权限、不投递通知、不激活窗口。
- 同一最终产物的 `--check-new-tab` 也 exit 0，[日志](../../target/verification/architecture-review-20261105/native-new-tab.log)确认新建／恢复／重开 Tab 路径通过；新增创建前后失败与复用 InputID 断言由 composer owner 负责。复用旧任务包已通过的本地 renderer、大纲几何和完整窗口重开证据，没有把模型检查声明为实际系统通知点击或原生 scene 创建回放。
- 旧 snapshot 没有设备身份，无法从会话 ID 自动判断归属；保留文件，只凭显式 scene UUID 恢复，不参加默认 latest 回退。失去原 scene 身份的记录需要用户明确映射本机／远端后补上身份。本次未读取或迁移真实记录。
- 没有触碰共享 composer 的 InputID／ContextKey，windowDevice 只属于本机呈现。未 commit、重装／退出日常应用或启动 Windows UI。
