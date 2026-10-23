# iPad 最小控制端

用户授权最快实现基础 iPad app，能力以 Android 控制端为范围，兼容 iPhone；避免过度实现、保护与验证；用户随后授权提交本任务改动。实现和构建入口已完成，平台说明归 [iOS README](../../app/ios/README.md)，产品决定归 [S48](../../docs/sources.md#s48)。

采用单一 SwiftUI 通用 iOS target，最低 iOS／iPadOS 17；iPad 分栏、iPhone 紧凑导航。经 remote-only UniFFI 复用配对、列表、增量转录、模型、发送、停止、交互与 agents 链接。没有本地 Harness、Node 或新远控协议。Markdown 使用 Mac 已采用的 swift-markdown。已有其他任务的远控与 Android/Mac 修改保持原状；本任务只为许可生成器增加可选 Rust target。

2026-10-23 完成必要验证：

- arm64 模拟器和 arm64 iOS 设备 Debug 构建均成功。设备构建使用 `CODE_SIGNING_ALLOWED=NO`，没有声称签名或安装完成。
- 独立 Velune iPad 模拟器以 `--preview` 启动合成会话，未打开远控端点、读取真实状态或调用模型。Debug 断言验证增量快照保留与删除消息、工作过程与回复分割不重叠。启动发现的表格单元格 formatter 崩溃已修复，再启动正常。
- 已观察 iPad 原生分栏、用户消息、工作过程折叠、Markdown 表格、代码块和输入区域。截图为 `target/ios-preview.png`；构建日志为 `target/ios-build.log`、`target/ios-device-build.log`。截图使用合成内容，不能证明真实通信闭环。
- 脚本语法、相关文档相对链接与标题锚点、`git diff --check` 通过。

本机 Cargo registry 的符号链接触发 Askama 模板路径错误，构建时使用实际缓存目录 `CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo`，未修改外部源码。可复现构建入口见 iOS README。

未验证真机签名安装、相机扫码、真实配对及远控会话闭环；未单独运行 iPhone 模拟器。已有图片只显示附件标识，搜索限于已加载会话；附件发送与完整端设置不在本次范围。没有扩大测试矩阵或访问真实凭据、会话。

2026-10-23 22:40（Asia/Shanghai），用户要求构建到其 iPad。使用本机已有 Apple Development 身份与团队完成设备签名构建，经 devicectl 成功安装到用户的 iPad Air 11 英寸（M4）并正常启动 `app.velune.ios`。此次使用普通应用入口，不是合成预览；未配对桌面端或读取真实会话。签名构建日志为 `target/ios-install-build.log`。此前“未验证真机签名安装”的限制已解除，扫码与真实远控闭环仍未验证。
