# 在 Velune 中使用 xURL

xURL 指 Xuanwo/xurl，访问 `agents://` 会话，不是 X/Twitter CLI。

先调用 `list_agents`。每个运行时返回 `runtime_id`、位置，以及可用时的 `xurl` 调用信息。只有 `same_machine_as_you` 为 true 且 `xurl.executable` 非空时，才可从当前 shell 调用该绝对路径。传入 `xurl.environment` 中的全部环境值，按 shell 规则逐项引用；不要修改全局环境或在别的机器使用该路径。它们绑定目标实例的 HOME、配置目录与 Harness 可执行文件，不含凭据。不要省略它们而落回调用方自己的配置目录。`xurl` 为空时使用 Velune MCP；不要声称官方 CLI 已安装。SSH 目标需要在目标机安装官方 xurl，不能运行本机二进制。

官方 CLI 的读写形式如下；以 list_agents 返回的绝对路径替代示例里的 xurl，并按目标实例设置环境。

```sh
xurl 'agents://codex?q=refactor&limit=5'
xurl 'agents://codex/019c871c-b1f9-7f60-9c4f-87ed09f13592'
xurl -I 'agents://codex/019c871c-b1f9-7f60-9c4f-87ed09f13592'
xurl 'agents://.?q=refactor&providers=codex,claude'
```

向用户展示会话时，把地址写成 Markdown 链接，例如 `[查看会话](agents://codex/019c871c-b1f9-7f60-9c4f-87ed09f13592)`，点击即可在 Velune 打开。不要要求用户复制地址再粘贴查找。

查询输出的地址用于进一步读取；`-I` 查看元数据与子会话地址。路径查询会跨多个 Harness 读取当前环境中的配置根，不代表 Velune 所有配置实例；多实例或远端应先逐实例定位。Pi 使用独立 sessionDir 时，官方 CLI 可能找不到，应使用 MCP。

Velune 管理的会话通过 MCP 查询和继续：`resolve_agent_uri(uri, runtime_id?)` 返回匹配及读取失败；`read_conversation(conversation_id, runtime_id?, offset?, limit?)` 分页读取文字历史；`send_message(conversation_id, runtime_id?, message)` 发送，随后 `wait_conversation`。这些 conversation_id 参数也接受具体的 agents:// 地址；歧义时指定查询返回的 runtime_id 或完整 conversation_id。工具继承原有忙碌、审批和子会话限制，不会从 URL 读取执行参数。Velune 的 URI 查询只按标题匹配 q；官方 CLI 支持更广的搜索、路径、角色和 Pi 条目查询。

官方 CLI 的写入使用 `xurl 'agents://codex/<id>' -d '消息'`，在 collection 地址上使用 -d 则创建会话。这会直接启动独立 Harness CLI，使用它的原生提供商和认证；不会沿用 Velune 模型路由、审批窗口、取消与协作消息来源。对 Velune 中的会话优先使用 send_message；仅在用户明确需要独立原生执行、已确认目标不在运行且原生认证可用时使用 -d。对超时或未知结果不要自动重发，先重新读取会话核对。不直接继续原生子 Agent，不自动安装或读取凭据。

来源：https://github.com/Xuanwo/xurl/tree/24eedb2ff2b141b025efa1a1658fd7f3e88cdd01 。这是 Velune 的接入指引；官方 CLI 不被替换或包装为同名命令。
