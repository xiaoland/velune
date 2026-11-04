# 分层验证记录

所有数据均合成，CLI 仅使用已安装的公开二进制。没有真实账户请求、真实任务启停或日常应用重装。临时日志根为 `/tmp/velune-control-investigation/`，脚本不入库。

## 原生适配器与应用行为

以下证据未受任务 UI 改造影响，直接复用。

- `application-e2e-verified.log`：实际 application release dylib + Codex 0.159.3／合成 ACP。首屏控制检查、外部空闲 writer 提前锁定、服务端拒发、释放无需转录写入即可恢复；回环 Responses 完成一轮后实际 native writer 持锁而本端可编辑。
- 同一 E2E：任务派发、忙时等待／释放执行、独立 Git 工作树、实际 cwd 记账、重启不重放、暂停幂等导入及来源重启用保护、删除引用运行时不被任务阻挡。HOME、CODEX_HOME、会话、Git 项目、上游全部隔离合成。
- native flock 单例、legacy 历史投影单例通过；原宿主 SQLite 的 ACTIVE／IN_PROGRESS 核对合成测试通过。
- iOS 模拟器 build；Android assembleDebug + lintDebug；Windows 隔离 worktree build 均通过。Windows 没有原生视觉验收，不能据此宣称界面体验已通过。任务管理窗口当前只在 Mac 提供。本轮 win-ws SSH 连接被关闭，没有重复 Windows 构建。

## 调度核心与应用接口

- `schedule-ux-core-tests-final.log`：10 项通过。新增每周构造与实际未来触发一致、修改时区跨 DST、复杂规则无损往返、无效执行定义仍可暂停；原六项继续通过。
- `schedule-ux-application.log`：实际 application 局部检查通过。覆盖独立启停持久化、目标删除后暂停、导入模型／推理映射、交接的启用与运行门禁、重导入 revision 保护、确认交接后仍暂停。没有派发任务指令。
- 严格 clippy：application／agent-runtime／bindings 的 all-targets、`-D warnings` 退出 0；最后仅移除临时原生事件打印，无业务变更。
- 最终 SVC Mac build：`539dc72c-5e94-47bd-8f96-61599eadb590`，2026-11-02 10:10:15 UTC 退出 0，日志 `delivery-macos-build.log`。

## 原生交互与布局

在唯一 bundle `local.velune.runtime-check-20261102` 和 `local.velune.click-check-20261102` 验证，没有操作日常应用。

- 外部 writer 持有时草稿保留、不可编辑但可选择复制，发送和附件入口禁用；释放后恢复且不自动发送。
- bottom drawer 在 1060×760 窗口的顶部边界始终 y=539；空闲／执行、空／18 行草稿、控制锁定／释放均不跳动，文本内部滚动。
- 列表双击 B 仅重命名，取消后 A 仍在；再次单击已选 B 后标题／正文转 B；原生多选不导航，键盘即时、Option 新标签和拖拽到无效目标不产生延迟导航。
- 原生追踪日志 `native-list-press-events.log` 定位 NSTableView 自己的 tracking loop 消耗 mouse-up。最终旁观 mouse-down 后异步读取原生 selection，不替换 table 的 selection／primary action／drag。临时位置和事件打印已移除，复用此前行为证据。

## 任务 UI 关键闭环

Advisor 两次只读评审的意见用于设计与复核，不把意见本身计作运行证据。以下均已在合成实例观察。

- 无需输入 RRULE 创建每周一／周五 09:00 任务，未来三次随选择变化；保存明确暂停，启用后 next=2026-11-06 09:00 与预览相同，随后立即暂停。
- 复杂规则原样呈现；无效规则禁保存；暂停保留无效文本；切换任务弹丢弃确认，取消后草稿完整。编辑期间刷新不打断输入。
- 故意清空名称后保存失败，刷新仍保留错误和草稿；取消后原定义完整。保存／取消退出编辑均通过。
- 导入并列显示原模型 synthetic-old-model 与实际默认模型，差异明确 low→high；确认覆盖后仍暂停，两个执行入口因未交接而禁用。
- 仅留任务窗口时能新建并打开 synthetic loopback turn；关闭新窗后再次打开仍成功。只恢复任务窗口的进程也直接载入运行时、会话标题与模型配置，通知／配置读取不依赖会话窗启动。
- 已查看概览截图的信息层次与主要操作位置。没有把所有显示尺寸、完整主题组合和真实账户流程视为已验收。

原生验证曾发现取消编辑的 SIGTRAP：17:34:40 的隔离实例报告触发栈为 `BindingOperations.ForceUnwrapping.get(base:)`。已改为代际隔离、退出期间仍可读取的显式草稿 Binding，并以上述保存／取消／切换重新验证。结果路由也排除了 SwiftUI 仍保留的已关闭窗口模型。

## 交付边界

两个隔离应用与临时辅助进程均已关闭。无提交，无日常重装。权威产品行为见 [定时任务 PRD](../../docs/prd/scheduled-tasks.md)；实现授权不等于用户已接受成品体验。


## 用户截图反馈后的 composer 布局复验

此前只检查抽屉外边界，不能据此判定锁定态的完整布局合格。用户 2026-11-02 的截图暴露出单行锁定提示撑满辅助滚动区、只读输入仍显示编辑焦点框和发送占位。本轮保留前述核心／应用业务证据，只重新验证受影响的 Mac 呈现与原生编辑边界。

修正为：锁定状态在既有操作行显示，点按可读完整原因；取消上方状态说明带，锁定不改变编辑框分配；焦点框只在可编辑状态显示，占位按只读原因呈现。真正的附件／队列区按内容高度限制到 64pt。原生粘贴、拖入和附件移除禁用，但文本保持可聚焦、可选择复制，键盘“转到输入区”继续可用。

合成 fixture `--preview --preview-composer-access --check-dock-areas` 中 A 空闲、B 外部占用、C 运行。已实际查看这些截图与 AX 状态：

- 1060×760 窗口，抽屉顶部 y=539；无附件时编辑框内边界约 x=257…842、y=550…715，空闲／锁定／运行中相同。锁定时没有额外说明带、编辑焦点框或发送按钮。
- 空草稿、单行草稿与 18 行长草稿；长草稿出现内部滚动条，不扩大抽屉或挤走操作行。隐藏左侧区域后，760 宽文本框也已检查。
- 锁定后单行草稿全文保留，原生全选成功；Copy 可用，Cut／Paste 禁用；尝试粘贴后草稿未变。点按“会话已锁定”可查看完整原因。
- 一份合成文本附件仅占约 36pt 的内容高度，不再撑满 64pt。文件面板关闭后自动化曾失去窗口定位；采样显示主线程正常等待事件，正常重开后附件仍在，未把此情况判为产品布局死循环。

最终构建已补验 Cmd+3 聚焦只读文本、Cmd+A 全选草稿，以及附件移除按钮 disabled；最后再次查看 760 宽空锁定截图。SVC build 94af35c6-da2f-4a37-88da-4fb809c62400 通过，日志 composer-locked-layout-delivery-build.log。只声明这些具体条件，不宣称覆盖所有布局组合。预览已正常退出。
