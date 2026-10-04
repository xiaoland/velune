# Velune Mac app

本 unit 使用 SwiftUI／AppKit 的原生窗口、会话列表、聊天投影和设置界面。品牌只出现在图标与少量内容细节中。

app 通过生成的 UniFFI 接口消费 application 的完整用例，不解析 Pi RPC，不自行启动网关，也不承担跨包失败补偿。Keychain 与秘密解析 shim 属平台设施；应用配置根为 `VELUNE_HOME`，默认 `~/.velune`。Pi 拥有真实会话持久化。

构建与本地安装入口为 [build-macos.sh](../../scripts/build-macos.sh)。真实登录、提供商调用和体验由用户验收；静态检查与隔离合成验证归 [开发说明](../../docs/development.md)。
