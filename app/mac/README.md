# Velune Mac app

本 unit 使用 SwiftUI／AppKit 的原生窗口、会话列表、聊天投影和设置界面。品牌只出现在图标与少量内容细节中。

app 通过生成的 UniFFI 接口消费 application 的完整用例，不解析 Pi RPC，不自行启动网关，也不承担跨包失败补偿。应用配置根为 `VELUNE_HOME`，默认 `~/.velune`；提供商 API key 由 application 私有配置保存，OAuth 保留原来源刷新。各 Harness 拥有真实会话持久化。

构建与本地安装入口为 [build-macos.sh](../../scripts/build-macos.sh)。真实登录、提供商调用和体验由用户验收；静态检查与隔离合成验证归 [开发说明](../../docs/development.md)。

运行时设置展示 family、精确版本类型与配置实例，当前提供 Pi 1.0.2、Codex 0.159.3 和 DeepSeek Harness 0.2.0-rc.2。同类型可保存多个实例，工作目录属于具体会话。App 只消费通用消息与类型化权限／回答交互，不解析 ACP 或 app-server envelope；选择、回答和取消通过生成接口交给 adapter。运行时要求外部 Node 22.19+ 的绝对路径；Codex 与 DSH CLI 由用户安装并配置。添加实例后即可浏览会话，新建表单选择实例／目录／模型，继续历史时自动准备。恢复 Codex／DSH 缺少提供商身份的模型选择须明确指定，不宣称恢复上次 Velune 路由。

构建前运行 Pi 与 runtime-support 两个安装脚本，huihua 依赖及许可证随 bundle 打包。具体步骤见 [开发说明](../../docs/development.md#原生运行时与历史读取)。构建产物不等于已安装或用户验收，最终状态以任务记录为准。

会话内容消费 Rust 的类型权威，Mac 只派生展示行与展开、滚动状态。用户内容在右侧气泡，assistant 正文及工具左对齐；系统通知居中，工具输出不按 LLM Markdown 解析。Markdown 采用固定 Swift Package Manager 依赖，许可证随应用分发；不恢复手写 fence 解析。懒布局使用稳定行身份，打开／发送到底，阅读历史时暂停流式跟随。当前实施与验收见 [展示任务](../../tasks/conversation-presentation/packet.md)。

Mac 由根目录 [Package.swift](../../Package.swift) 定义独立 SwiftPM 产品，依赖固定 MarkdownUI 2.4.1；[Package.resolved](../../Package.resolved) 锁传递依赖。build-macos 先生成 UniFFI 模块，再以 SwiftPM release／warnings-as-errors 构建原生 app，并打包 `app/mac/Licenses` 与解析锁文件。Swift 包不依赖领域 Rust 包源码，只链接对应生成绑定与库。

会话上下文菜单按运行时能力提供重命名和永久删除确认。选中目标、待加载身份与已加载详情分别管理；加载时显示进度，暂停其它选择并隔离旧轮询，失败恢复原会话。操作交给生成绑定和运行时执行，不建立 Mac 标题覆盖存储。隔离验收见 [会话管理任务](../../tasks/session-management/packet.md)。
