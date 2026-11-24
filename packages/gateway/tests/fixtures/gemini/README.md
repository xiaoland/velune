# Gemini 协议 fixtures

五份测试数据原样取自 Vercel AI SDK，采用 Apache License 2.0。`NOTICE` 保留上游的版权声明，`LICENSE` 收录许可证全文。 [固定版本的上游 fixtures](https://github.com/vercel/ai/tree/8956ca55ed9c31b4b7680621a10ad0269f9629b1/packages/google/src/__fixtures__)

`.json` 直接作为回环 HTTP 响应，`.chunks.txt` 的每个非空行在回放时包成 SSE data 帧，保持 Gemini 的业务终态和用量字段。测试凭据由本地测试生成。
