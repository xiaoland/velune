# Velune Mac app

本 unit 使用 SwiftUI／AppKit 的原生窗口、会话列表、聊天投影和设置界面。品牌只出现在图标与少量内容细节中。

app 通过生成的 UniFFI 接口消费 application 的完整用例，不解析 Pi RPC，不自行启动网关，也不承担跨包失败补偿。应用配置根为 `VELUNE_HOME`，默认 `~/.velune`；提供商 API key 由 application 私有配置保存，OAuth 保留原来源刷新。各 Harness 拥有真实会话持久化。

构建与本地安装入口为 [build-macos.sh](../../scripts/build-macos.sh)。真实登录、提供商调用和体验由用户验收；静态检查与隔离合成验证归 [开发说明](../../docs/development.md)。

运行时设置展示 family、精确版本类型与配置实例，当前提供 Pi 1.0.2、Codex 0.159.3 和 DeepSeek Harness 0.2.0-rc.2。同类型可保存多个实例，工作目录属于具体会话。App 只消费通用消息与类型化权限／回答交互，不解析 ACP 或 app-server envelope；选择、回答和取消通过生成接口交给 adapter。运行时要求外部 Node 22.19+ 的绝对路径；Codex 与 DSH CLI 由用户安装并配置。添加实例后即可浏览会话，新建表单选择实例／目录／模型，继续历史时自动准备。恢复 Codex／DSH 缺少提供商身份的模型选择须明确指定，不宣称恢复上次 Velune 路由。

构建前运行 Pi 与 runtime-support 两个安装脚本，huihua 依赖及许可证随 bundle 打包。具体步骤见 [开发说明](../../docs/development.md#原生运行时与历史读取)。构建产物不等于已安装或用户验收，最终状态以任务记录为准。
