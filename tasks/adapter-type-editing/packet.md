# Adapter Type 可编辑与退役 Codex0.159

用户已确认：Adapter Type相互独立，不建设升级/继承/兼容关系；现有实例可编辑类型，保留实例身份和通用执行环境，各类型专属表单独立；在途执行沿用快照，后续用保存后的新配置。本次删除0.159并显式转换旧数据到166。必要真实迁移已持续授权，不逐次确认。日常重装仍由用户在应用外执行，不抢鼠标/前台/不跑WindowsUI。

## 当前与实现边界

- Mac仅UI禁用类型选择；核心upsert已有类型变化失效/在途隔离逻辑。需核实不同类型切换不会误把旧表单字段发给新类型，不引入类型关系。
- 已有会话关联含runtimeTypeId；普通类型切换不自动改写原生历史归属。本次退役是一次性显式迁移，清理旧枚举与适配分支。
- 工作树有用户另一批fork修改（见target/verification/adapter-type-editing-20261105/preexisting.patch），全部保留，不能误纳为本任务改动或重置。

## 所有权与验收

retire_codex159负责Rust适配器删除和引用；editable_adapter_ui负责各端表单与枚举引用及Mac窄验；root负责显式迁移、文档、核心保存边界与集成。

验收仅新关键路径：现有实例换类型/换回草稿、身份/环境保持；在途保存到新类型不改当前执行；迁移备份/身份/关联保留，实际166查询隐藏attached记录复用上轮证据；共享编译和受影响客户端构建。无依据不重跑全套发送或UI。

## 已获证据

- Rust旧适配器已移除，32项agent-runtime单测、1项旧类型解析拒绝测试、application release/all-targets check通过。原Mach-O首次加载停留后正常结束，非断言失败；没有扩大为全套测试。
- 一次性脚本新增CLI核实/选择、全部旧关联含孤立段转换，保留ID/HOME和私有备份；窄验预览不写、缺匹配版本整批拒绝、私有备份、身份/设置/孤立关联保留、幂等、完整runner后续输入转换通过。探针版本字符串在单测中mock，实际166版本/原生列表语义复用此前已验证证据；本机新临时shebang直接执行有加载延迟，未把它放宽成成功。
- 实际Pi+回环提供商429时保存同ID的新Codex166类型，Pi原轮重试产生成功assistant stop记录。最初验证器等待旧Pi会话can_send导致超时，这不属于承诺（新类型无法读取旧原生格式时应不可用）；核对原生成功结果后仅补新执行路径，没有重跑Pi原轮。
- `next-type.log`：复用已保存状态，新执行实际写入Codex会话并收到合成回复，实例ID/显式HOME保持，核心重开后166类型仍保留。没有真实模型调用。

## 真实目录当前决定

本机正常退出请求超时，应用仍持有runtime.lock，故没有写真实目录。用户随后明确选择“先保持运行，稍后迁移”；此决定只延后真实迁移，不中止源码与隔离验收。不得再次请求退出或绕锁写入。待用户安排迁移/外部重装时执行一次性脚本；当前日常安装继续原配置，因此同名内部线程展示问题尚未对该安装解除。

## 集成与客户端证据

- Mac首次SVC build（parent日志macos-build.log，执行子日志d736e565-2a9c-49d1-a428-a36eec82f47b）Rust及绑定通过，Swift暴露原有fork改动5处契约未接齐；只修预览/消息详情调用，不撤回原功能。随后仅执行原build-macos.sh的Swift到末尾完整尾段，复用新原生库/绑定；resources、许可证、图标、manifest、完整签名均完成，mac-swift-build.log通过。
- 初次原生runtime-editor-only通过：真实NSPopUpButton enabled/title、各类型表单/ACP launch/新会话默认往返、真实save持久化、ID/name/gateway/enabled/SSH/HOME保持、旧LastChosen清零；root已看runtime-editor.png，无前台或鼠标注入。未将直接Binding驱动声称真实菜单点击。
- 最后反例复核发现保存回包重读后改草稿的竞态；已改保存时捕获type和新会话选择，新增save后立即改回原ACP/lastChosen的条件。仅补Mac Swift增量和本窄验；不重复核心或其它客户端。Windows已有await前局部payload，无同类回读。
- iOS最终remote-only原生/Swift绑定/arm64模拟器app通过；首轮仅补既有fork字段的preview构造。Android原生/Kotlin绑定/assembleDebug/lintDebug通过。Windows隔离Rustrelease/新C#绑定/dotnet publish/资源许可证打包通过，并已安全解除junction、清理worktree和上传临时物，日常remote源码/HEAD及缓存保留。没有安装/启动设备UI，不声称Windows交互或iOS真机签名验收。

## 最终结果

- `mac-swift-build-race.log`完成最后Swift增量、完整资源打包与deep/strict签名验证；`runtime-editor-race.log`通过。保存后、回包前切回原ACP与lastChosen，真实落盘仍为点击保存时的Claude类型及其选择，LastChosen清空。仅补这一条编辑器入口，没有重复其它UI、在途运行时或跨平台构建。
- 最终Markdown相对链接/锚点168个检查通过；git diff --check通过。Mac、Windows、iOS模拟器、Android四端构建均有最终证据。
- 源码实现和构建结果已就绪；日常安装未替换，真实迁移因用户选择保持运行而留待稍后。当前运行中的旧安装仍使用0.159，不能声称其同名条目已消失。外部重装流程会在新应用启动前执行本次显式转换。
- 本任务没有提交；工作区含用户并行任务改动，全部保留。
