# 运行时 MCP 注入证据

检查日期为 2026-10-11。此记录归属 agent-runtime 注入与传输边界；application 的父子任务调度及真实模型体验不由此记录证明。所有操作使用临时 HOME、临时项目和合成 loopback MCP 服务，没有读取真实认证或历史，也没有调用真实模型。验证脚本临时放在忽略的 target 目录，未加入自动测试或 CI。

## 已验证

通过新 Rust `NativeSession`／Pi `Client` 分别连接实际 Codex 0.159.3、Claude Code 2.1.286、Pi 1.0.2 和 DSH 0.2.0-rc.2，创建会话后正常关闭。四者均用各自指定的短生命周期 Authorization 头向合成服务完成 `initialize`、`notifications/initialized`、`tools/list`，实际发现工具。没有发送模型 turn；因此该观察证明原生 MCP 客户端接入，不单独证明模型会决定调用工具或完整委派。

Codex 的独立隔离实验在合成原配置中先定义一个 MCP 服务，再通过 `thread/start.config.mcp_servers` 注入另一服务。两者都完成初始化和工具读取，`mcpServerStatus/list` 同时包含两个名称，原 config.toml 字节未变。Claude 的独立隔离实验先加载合成原服务，再执行 `mcp_status`、`mcp_set_servers`、`mcp_status`；原服务仍在列表中，新增服务变为 connected 并公布工具。生产实现仅经控制 stdin 传递 token，不将其放进 CLI 参数或持久配置。

通过合成 ACP peer 分别公布和省略 `mcpCapabilities.http`，实际 Rust `Client` 的 `create`、`load`、`resume` 三条路径都携带了 MCP 服务。HTTP 模式直接到达合成服务；stdio 模式实际启动投送的 Python relay，完成初始化与工具读取。正常关闭后 peer 的 relay 子进程退出，Rust Client 释放后所记录的临时 relay 路径已不存在。

relay owner 使用独立合成 HTTP peer 验证：初始化 session/version 头转发、等待期间并发状态／取消、20 个并发响应、202／204 通知与客户端响应、401、SSE 拒绝、无效 JSON、响应 ID 不匹配、原 JSON-RPC 错误保真、UTF-8 和数值 ID 0。诊断未回显合成 token 或正文。stdin EOF、stdout 断开与绝对 deadline 分支均已验证能停止传输；relay 不代替 application 发送业务取消。

真实 SSH 验收使用用户已授权的 WSL 目标、现有 known-host 校验和一次性隔离 HOME／项目。目标为 Linux 6.18.40.1、Python 3.13.5；OpenSSH 连接、远端进程与反向转发均为实际执行，ACP Agent 和本机 MCP broker 为合成 peer。通过 Rust `Client` 的 `create/load/resume`，不声明 HTTP 能力的远端 peer 三次实际启动投送的 relay，并完成初始化和工具读取。本机 broker 验证了 Bearer、Accept、路径与 query，以及初始化后传递的 `Mcp-Session-Id` 和协商的 `MCP-Protocol-Version`。Python 与 relay 路径为目标机器的绝对路径；本机不存在该 relay 路径，远端分配的端口仅在 `127.0.0.1` 监听。

另一次相同 WSL 路径发送合成 `tools/call`，本机 broker 验证请求参数与上述全部 HTTP 头，返回文本 content、`isError: false` 和 structuredContent。远端 peer 核对原请求 ID 与完整结果后才允许会话建立成功，证明调用与结果实际经过 SSH 反向转发和 stdio relay 往返；工具本身只回显合成参数，没有真实业务副作用。

SSH 生命周期观察区分正常关闭与对象释放：`shutdown` 后 ACP peer 和全部 relay 进程已退出，`Client` 释放后远端 relay 目录与反向监听均消失。换会话时也观察到了 `session/close`，最终正常 EOF 到达。验收完成后本机和远端唯一临时 fixture 均已删除并核实。此路径没有经过 App／UniFFI，也没有调用真实 Harness 或模型；不以旧 SSH 结果替代此次 MCP header、投送路径和清理证据。

## 实际 application MCP 入口

使用本轮生成的 Python UniFFI 绑定与新构建的 `libvelune_bindings.dylib`，在两个独立临时 application HOME 中调用真实 `create_collaborative_conversation`。合成 ACP parent 从 `session/new` 接收宿主注入的 HTTP 配置；票据只保留在内存和权限为 0600 的自有临时捕获文件，没有输出。此次 HTTP 服务由实际 application／rmcp 提供，没有替换为合成 broker。

两个 scope 都通过以下观察：正确 Bearer 的 `initialize` 成功并报告 Velune 协作服务，`tools/list` 返回七个协作工具；缺失票据、错误票据和另一个仍活动 scope 的票据均返回 401；带正确票据但伪造外部 Origin 或 Host 的请求均返回 403。`spawn` 的未知目标、额外 cwd 和额外 binary 参数都返回工具错误，随后查询确认没有分配子任务。关闭 scope 后，其原 endpoint 已拒绝连接。两个 scope 的临时 HOME 和捕获文件均已清理。该验收没有启动子任务、调用模型或修改产品源码；父子业务生命周期另由完整协作验收覆盖。

## 依据与边界

- ACP stable v1 要求支持 stdio MCP，HTTP 由 `mcpCapabilities.http` 协商；服务配置随 `session/new/load/resume` 输入。来源为已保存的官方协议参考 `af9d8f5d3852f8c3c774b81f7366e6707e0fd5e1`，以及 [官方 MCP 会话配置](https://agentclientprotocol.com/protocol/v1/session-setup#mcp-servers)。
- Pi 1.0.2 的公开 SDK 与安装包文档确认 SDK session 需要显式装配 `createMcpExtension`，`registerMcpServer` 注册只作用于会话且不保存。MCP 调用仍通过原生工具 pipeline 与权限扩展。来源为 [Pi SDK](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sdk.md) 和 [Pi 扩展](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md#mcp-servers)。
- Claude 固定版本已实际确认 `mcp_set_servers` 动态控制。该控制也见官方 [TypeScript SDK 变更记录](https://github.com/anthropics/claude-agent-sdk-typescript/blob/main/CHANGELOG.md)。不增加全局允许工具或跳过权限选项。
- DSH 固定发行包的 ACP 模块把标准 MCP 配置挂在 Agent scope，stdio 与 HTTP 均受支持；启动失败会拒绝建立会话。生产代码只使用已声明的 HTTP 能力，不另读私有配置。

本机四个真实 Harness 的模型调用、权限弹窗和跨 Agent 任务完成另需父子闭环证据。relay 要求目标 Python 3，broker 必须使用 JSON 响应；SSE、服务器主动请求和外部认证不在此次 relay 范围。
