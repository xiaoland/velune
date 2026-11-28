# Gemini 原生 API 接入

让 Velune 通过 Google Gemini Developer API 调用 `generateContent` 和 `streamGenerateContent`，支持同协议透传及现有 Harness 经网关使用。授权已覆盖实现。验收使用公开 fixture 和回环 mock，不接触真实账户或模型服务。

现有提供商模板、网关、Pi 和用量统计已接通。Gemini 原生请求保留思考签名；跨协议转换会记录无法保留的签名。

## 验证

- 回放 Vercel AI SDK 的固定版本数据，覆盖 JSON、SSE、签名工具续轮、用量和 429。网关 dispatch 回归 19 项通过，原生 HTTP 单元测试 6 项通过。 [仓库内的 fixtures](../../packages/gateway/tests/fixtures/gemini/README.md)
- 在隔离目录用本机 Pi 1.4.0、新生成的 Python 绑定与合成 API key 驱动完整调用。Vercel 正文录制计量为 217 token。Google JS SDK 的自动函数调用续轮响应先报告 101，最终报告 64；Velune 统计为 64。具体型号与 `gemini-flash-latest` 别名均发送了 `thinkingLevel: HIGH`。 [Google JS SDK HTTP 录制](https://github.com/googleapis/js-genai/tree/4013660c2f772027a3ef06f2143ea1bcb2af35e3/test/system/recordings)
- Rust 共享绑定编译检查与 Pi 提供商导入预览通过。
- macOS 和 Windows 开发构建通过。Mac 的独立 Swift 验收用生成绑定读取模板，并确认模板、API key 认证、默认地址和原生协议选项。Windows 隔离实例的浅色、深色界面均能选择 Google Gemini API 模板，显示正确协议和默认地址，随后正常退出。
