# 侧聊

2026-10-29 用户解除功能冻结后要求（[S56](../../docs/sources.md#s56)）提供侧聊，即 Claude Code 的 `/btw`：上下文优先用运行时自己的侧聊能力，没有时在运行时内原生 fork 出不保存的临时副本；多轮，关闭即丢弃；侧聊中能做什么由运行时自身决定，默认不另加限制；首个 beta 前只做 Mac。产品规则见 [侧聊](../../docs/prd/conversations.md#侧聊)。

完成依据：Mac 上 Claude Code、Codex 与 Pi 的会话能打开侧聊、多轮提问、关闭后不留下原生会话或 Velune 记录，原会话的历史、下一轮选择与发送队列不变；原会话运行中也能使用。DSH、ACP 与未首发的会话不提供入口。使用临时 HOME、合成上游与回环请求验证，不读取真实配置、凭据或会话，不调用真实模型。

## 协议证据（2026-10-29，本机安装版）

| 运行时 | 证据 | 方式 |
| --- | --- | --- |
| Claude Code 2.1.292 | print 模式 stream-json 接受控制请求 `{"subtype":"side_question","question":…,"history":[{"question","response"}]}`，以 `control_request_progress` 报告进度，最终 `control_response` 带回答。隔离 HOME、合成 key、不可达端点下实测请求被受理并开始调用 API。`/btw` 本身是交互命令，内部同一实现，不能使用工具。 | 每次提问以 `--resume <会话> --no-session-persistence` 起子进程发送该请求，history 由 Velune 保存。最小支持版本 2.1.286 是否已有此请求未验证。 |
| Codex 0.159.3 | TUI 的 `/side`（“start a side conversation in an ephemeral fork”）由 app-server 的 `thread/fork {ephemeral: true}` 与 `thread/inject_items` 组成，注入“此前历史仅作参考”的边界与开发者说明；app-server 没有专门的侧聊方法。 | 用独立 app-server 进程做同样组合，之后按普通线程发送、取消与审批；关闭时退订并结束进程。 |
| Pi 1.0.2 | 没有侧聊；SDK 有 `SessionManager.inMemory(cwd, options, entries)`。 | 以原会话条目建内存会话，不写盘。 |
| DSH、通用 ACP | 没有侧聊或 fork。 | 不提供。 |

## 实现

- agent-runtime：`NativeSession::open_side`（Claude Code 侧问、Codex 临时 fork 加边界）、Pi `Config.side`（`pi_rpc.mjs --side` 内存会话），见 [agent-runtime README](../../packages/agent-runtime/README.md#侧聊)。Claude Code 侧问不能 `interrupt`，停止与关闭改为关闭子进程输入、让它自行退出（强制结束会留下 `sessions/<pid>.json`）。
- application：侧聊是带 `SideChat` 的独立 session，命令 `OpenSideChat`／`SendSideChat`／`CloseSideChat`，读取、取消与审批沿用会话命令；排除于列表、活动、可恢复、结束的 turn、来源跟随、协作 MCP 与网关轮次后的关闭，见 [application README](../../packages/application/README.md)。
- Mac：`SideChat.swift` 面板与入口，见 [Mac README](../../app/mac/README.md)。

## 验证（2026-10-29）

- 临时 Python 脚本（不入库）：临时 HOME、合成 Responses／Messages 回环上游，实际 Codex 0.159.3、Claude Code 2.1.292、隔离 Pi 1.0.2。三者都通过：侧聊带原会话上下文回答、第二问带上第一问的问答；原会话运行中也能使用；停止回答后仍可继续提问；回答中关闭即结束；侧聊不出现在列表、活动与结束的 turn 中，原会话消息不变；关闭后原生会话文件（含 Claude Code 的进程登记）不变、没有新增；原会话随后的请求不含侧聊内容。Codex 请求中有注入的边界；0.159.3 的 fork 不把 `developerInstructions` 放进请求（与其 TUI 相同的参数）。
- 隔离 Mac 副本（`--check-side-chat`，合成 Codex 夹具）：从列表打开侧聊、提问、回答显示在会话右侧面板、关闭。首次核对发现历史会话在应用重开后没有下一轮选择、侧聊落到运行时自己的提供商（向 api.openai.com 发出了无凭据请求，收到 401，内容只有合成文字）；改为由客户端传入下一轮的模型、推理强度与权限模式后复核通过，夹具另设 `OPENAI_BASE_URL` 指向不可达回环。
- `cargo fmt`、workspace clippy（all features、all targets）、agent-runtime／application 单元测试、Mac Swift 编译通过；相对链接检查无错误。
- 未验证：用指针实际点按菜单与 `/side` 补全（与“会话”菜单同一命令）；Claude Code 2.1.286 是否已有侧问。
