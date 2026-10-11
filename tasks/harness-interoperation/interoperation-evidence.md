# 真实父子 Harness 互操作验收

2026-10-11 使用 OpenCode 1.18.35 作为真实 ACP 父运行时、DSH 0.2.0-rc.2 作为真实 ACP 子运行时，经当前 `velune-bindings` 动态库和生成的 Python 绑定完成验收。模型响应来自本机合成 OpenAI ChatCompletions／Anthropic Messages 服务；没有登录、读取真实凭据或调用真实模型。

## 观察结果

通过 `VeluneApplication.create_collaborative_conversation` 创建父会话并授权一个 DSH target。OpenCode 的实际 provider request 包含 Velune 注入的 MCP 工具；合成 provider 动态查找带服务名前缀的 `agents`、`spawn` 和 `wait`，按序发出原生流式 tool call。OpenCode 执行这些工具，application 启动 DSH，DSH 请求到达 loopback Messages provider，并返回 `CHILD_DSH_RESULT`。子结果经 `wait` 回到 OpenCode 的 `role: tool` 消息；父继续请求模型，最终输出 `PARENT_AFTER_CHILD` 并进入 Idle。

父发送后，脚本在观察到父 provider 请求含子结果之前不调用任何 application 轮询 API。该断言证明子 worker 和 MCP 工具在前台不轮询时仍可完成委派。之后读取最终状态，确认一个 Idle 子任务、准确的结果文本、父最终正文及 Idle 状态。`child_snapshot` 在最终动态库上成功解码，消息 stamp 非零，连续读取不变消息时 stamp 稳定。早期产物发现的 `missing field stamp` 已修复并通过此断言，未作为待办遗留。

手动入口为 [manual-harness-interoperation.py](../../scripts/manual-harness-interoperation.py)，完整运行退出码为 0。脚本每次把指定动态库和绑定复制到新临时目录，避免旁边的旧动态库影响结果。它清空继承环境，为应用和两个 runtime 分配独立 HOME、XDG 与项目目录；macOS sandbox 限制出入站网络为 loopback，阻止读取用户目录、OpenCode 系统配置与 MDM 配置，仅为显式 Node 安装目录放行必要文件读取。DSH 的临时 Cordis patch 显式选择合成 Anthropic provider，OpenCode 只启用合成 provider；所有临时文件随验收清理。

## 复现与适用边界

先构建 `velune-bindings` 并生成 Python 绑定，准备上述版本的 OpenCode、DSH 与 Node。此手动入口要求 macOS `sandbox-exec`；它不安装运行时，不接入自动化测试或 CI。路径由调用者传入，例如：

```sh
python3 scripts/manual-harness-interoperation.py \
  --bindings <生成的-Python-绑定目录> \
  --library <本次构建的-libvelune_bindings.dylib> \
  --opencode <OpenCode-可执行文件> \
  --dsh <DSH-lib/bin.js> \
  --node <Node-可执行文件>
```

OpenCode 的工具权限在该隔离 fixture 内允许，用于触发合成 provider 指定的协作调用；产品注入没有增加全局允许工具或跳过权限参数。此结果证明实际 Harness 的协议、工具调用和父子结果闭环，不证明真实模型会自主正确拆分任务，也不代替真实提供商与 App 体验验收。App 子任务入口仍待实现。

官方依据为 [OpenCode ACP Support](https://opencode.ai/docs/acp/) 与 [MCP servers](https://opencode.ai/docs/mcp-servers/)。固定版本实测确认 ACP v1、HTTP MCP 能力和 MCP `2025-11-25` 初始化；临时注入名称会成为 OpenCode 模型工具名的前缀，因此 fixture 从实际工具表发现名称，不硬编码随机 scope 名。
