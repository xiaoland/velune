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

## 方案（待实施时细化）

- agent-runtime：每种运行时提供打开侧聊的入口，返回一个只存在于内存、不登记到列表的会话执行；Claude Code 的侧聊以侧问实现发送，其余复用普通轮次的发送、取消、审批与快照。
- application：侧聊以独立的临时会话 session 保存，不进入列表、活动、未读、草稿、下一轮选择、组织记录与 Velune 链接；网关、额度与分析照常记录其模型请求。关闭或应用退出时结束。
- Mac：会话视图里的侧聊面板（自己的回答列表与输入框），入口为输入区 `/side`（别名 btw）、会话菜单与命令面板；每个标签页最多一个侧聊。

## 验证

待实施后记录。
