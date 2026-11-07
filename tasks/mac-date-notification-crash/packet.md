# 零点系统通知导致消息列表崩溃

## 目标与边界

用户反馈Velune再次崩溃，沿实际报告定位并修复。必要源码修改和隔离实验在现有修复授权内；不退出、重装、前台激活日常应用，不操作全局输入、不读真实会话／凭据。其他会话的未提交改动保留。原工作区两组七Tab人工恢复仍按用户稍后统一重启的决定暂停。

## 已确认事实与判断

最新实际报告`Velune-2026-11-06-000005.ips`：2026-11-06 00:00:02，安装版PID4610（启动21:15:32）在后台thread19收到`__postAndResetMidnight`，同步进入`TranscriptTableController.typographyChanged`、重建可见行、`TranscriptTextView.show/setFrame`，触发NSISEngine／NSLayoutConstraint异常并abort。不是前一次composer绑定递归。安装版manifest基于073cbee且dirty，但同一错误路径仍在当前源码，不能只归因于版本未部署。

宏观边界是Foundation系统事件与UI执行域之间缺少显式调度。MainActor标注不会替Objective-C selector自动切线程；把日期环境变化映射为字体变化，还扩大了缓存失效范围。独立通知消费复核未发现第二条已证实的后台系统通知直接改AppKit几何路径，内部外观通知由MainActor发布，其余AppKit生命周期来源维持原有职责。

## 实现与当前下一步

消息列表controller持有日期环境订阅，覆盖日变更、时区、时钟与locale；用Combine显式receive到main后再处理，弱引用随controller寿命取消。日期变化仅使布局和可见cell配置失效，保留RenderedMessage、字体及历史高度估计，隐藏Tab延迟重排。外观变化仍拥有文本／字体缓存失效。不新增通用事件服务、持久日期状态或专项应用检查入口。

已完成Mac dev／release构建、签名验证及聚焦原生通知实验。没有增加永久回归harness，没有重复运行时／移动平台矩阵。部署仍由用户在应用外执行，未安装／重启真实应用；本次源码未提交。

## 根因的独立确认

统一日志00:00:01.992明确记录NSInternalInconsistencyException：布局引擎已由主线程访问，后续禁止后台线程修改。它与崩溃报告的后台午夜通知→selector→setFrame调用链相互印证，证据`exception-reason.log`。这是日期显示接入遗漏的线程契约，不能仅通过绕过AppKit异常或在NSView处加保护解决。

同类全局环境入口还为滚动条偏好加上main scheduler；这不是另一条已证实的崩溃。原生AppKit几何回调依赖同步处理，未批量改成异步。通知归属和缓存失效规则已写入Mac README。

## 分层验收结果

证据根`target/verification/mac-date-notification-crash`。

- `build-final.json`：最终dev源码构建／签名通过，SVC约68.6秒。
- `release-build.json`：仅Mac release完整构建／签名通过，SVC约704.4秒；这是新标准host-rust域的首次优化构建，后续复用其缓存。未为此再构建其它平台。
- `native-calendar-result.json`与`lldb-calendar.log`：在最终dev App的真实原生消息列表上，用4个后台线程分别发布日、时区、时钟、locale通知；实际处理4次全部在主线程。可见正文同一NSTextView、选择(0,2)、scroll y=28及正文视口位置y=8保持。
- 日期标签textStorage编辑次数0→1→2→3→4，均在main；实际系统时间未改，字符串可以保持相同。此观测证明日期布局重新应用，空handler／漏布局失效不能凭保持选择误过。
- 窗口隐藏且非key，应用未激活，合成进程exit0。只使用临时HOME／VELUNE_HOME及临时native dylib／LLDB脚本，没有修改系统时钟或发送全局输入。

临时验证复用了既有native Table fixture的初始化，但跳过了独立的旧catalog白盒helper。该helper仍直接改已退役的nextTurn私有派生字段，不能代表新共享目标准备用例；本次没有调整生产配置解析来迎合它，也不声称整个navigation fixture通过。实际Table准备后，临时RunLoop驱动只执行本次通知观察并退出，后续无关hover断言不纳入验收。此旧验证夹具需随原用例维护单独整理，和用户实际崩溃原因分开。

## 交付边界

修复产物位于`target/macos/Velune.app`，可供用户在应用外运行既有重装入口。已安装版和真实窗口数据未被替换；此前两组七Tab的人工身份映射仍按原暂停安排保留。没有用午夜等时等待代替可重复通知验证，也没有以编译成功代替运行证据。最后核对相对链接／锚点与diff空白。
