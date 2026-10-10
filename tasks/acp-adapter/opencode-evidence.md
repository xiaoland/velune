# OpenCode 原生 ACP 验收

2026-10-10，在 macOS arm64 上通过 Velune 的现有 UniFFI → application → `acp::Client` 接入 OpenCode 1.18.35。真实 `opencode acp` 完成了建会话、配置模型、向本机合成模型服务发送、关闭后发现和加载历史。没有修改 production 源码，也没有为 OpenCode 增加 Harness 分支。

## 版本与来源

验收二进制来自官方 npm 发布的 [`opencode-darwin-arm64@1.18.35`](https://registry.npmjs.org/opencode-darwin-arm64/1.18.35)，`--version` 和 ACP `agentInfo.version` 均返回 `1.18.35`。下载包的 SHA-512 与 registry 的 integrity 一致：`WlM5yNbT6OppyXBaTgnri0tnsi/axeLl16vlIrfIdjm4yEvuZVD0JBpBNOLBI21jig0Rw7/HRAO0XWokgJge9g==`。对应官方源码 tag [`v1.18.35`](https://github.com/anomalyco/opencode/tree/v1.18.35) 指向提交 `53d1eabb61e21162157817bf677da0a4ad3332e3`。

[官方 ACP 文档](https://opencode.ai/docs/acp/) 明确以 `opencode acp` 启动 stdio JSON-RPC Agent。[官方自定义 provider 文档](https://opencode.ai/docs/providers/) 提供 `@ai-sdk/openai-compatible`、`options.baseURL` 和显式模型配置方式。此次实际二进制接受 `provider`、`enabled_providers`、`model` 与 `small_model` 的配置，两个模型字段都设为同一个合成模型。

## 隔离边界

安装包、临时脚本和所有运行数据仅位于 ignored 的 `target/opencode-acp-probe/`。启动前清空继承环境，再显式提供独立 HOME、四个 XDG 目录、TMPDIR、配置文件和项目目录。未配置真实认证来源；provider key 仅为合成字面值。使用该版本的 `OPENCODE_DISABLE_PROJECT_CONFIG`、`OPENCODE_DISABLE_MODELS_FETCH`、`OPENCODE_DISABLE_AUTOUPDATE` 和独立 managed config 目录，配置只启用 `fixture` provider。

由于该版本仍会检查 macOS MDM 配置，额外通过 `sandbox-exec` 拒绝读取 `/Users`、`/Library/Managed Preferences` 和系统 OpenCode 配置目录，并拒绝非 loopback 网络访问。该限制是本次验收措施，不是 Velune managed HOME 的产品能力。隔离开关依据该版本的 [flag](https://github.com/anomalyco/opencode/blob/53d1eabb61e21162157817bf677da0a4ad3332e3/packages/core/src/flag/flag.ts)、[global paths](https://github.com/anomalyco/opencode/blob/53d1eabb61e21162157817bf677da0a4ad3332e3/packages/core/src/global.ts) 和 [managed config](https://github.com/anomalyco/opencode/blob/53d1eabb61e21162157817bf677da0a4ad3332e3/packages/opencode/src/config/managed.ts) 源码核对。

## 实际结果

| 验收项 | 实际观察 |
| --- | --- |
| 初始化 | ACP v1 成功；`agentInfo.name = OpenCode`，版本 `1.18.35`。 |
| 协商能力 | `load_session`、`list_sessions`、`resume_session`、`close_session` 为 true；`delete_session` 为 false。 |
| 认证描述 | 返回 `opencode-login`，提示在终端执行登录；本次未调用登录或 authenticate。 |
| 创建 | `create_conversation_with_selection` 使用 `RuntimeDefault` 成功，取得 OpenCode 原生 session ID。 |
| 配置 | 返回 `model` 与 `mode` 两个 select 配置项；模型值为 `fixture/fixture-model`，模式为 `build`、`plan`。 |
| 模型选择 | `runtime_models` 返回真实会话模型；`set_acp_session_config(model, Select(...))` 成功。 |
| 活动会话发送 | 创建、配置、发送在同一活动 Client 中连续完成；未经过关闭或加载。 |
| 合成上游 | 本机 HTTP 服务收到两次 `/v1/chat/completions` 流式请求，模型均为 `fixture-model`；projection 显示 `OPENCODE_ACP_SYNTHETIC_ANSWER`。 |
| 关闭与重开 | application shutdown 成功；重新打开 application 后，`list` 发现该原生会话，显式 `open_acp_conversation(id, cwd)` 回放合成答复。 |
| 重复加载 | 相同 session 连续加载两次，投影消息 ID 一致。 |
| 清理 | application、临时 HTTP 服务均关闭；结束后检查没有本次安装路径的 OpenCode 或 sandbox 子进程残留。 |

临时手动脚本为 `target/opencode-acp-probe/probe.py`，能力描述补采为同目录 `describe_peer.py`。主脚本退出码为 0，最终结果为 `PASS initialize/new/config/prompt/list/shutdown/reopen/repeated-load 2 loopback requests`。脚本不接入自动测试或 CI。

本次验证的是实际 OpenCode ACP 协议和原生持久化在合成模型上游下的闭环。没有验证真实供应商认证、计费模型、工具权限交互或取消；也未单独调用 resume。delete 不在该 peer 的协商能力内，未伪造支持。活动 Client 的连续使用由接口路径验证，未额外采样其 OS PID。App UI 未改动，接入仍由 [本任务 packet](packet.md) 标记待实现。
