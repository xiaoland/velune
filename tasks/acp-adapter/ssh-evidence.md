# 通用 ACP 的指定远端验收

2026-10-10，使用用户已授权的 `wsl.win-ws.localhost`，通过当前 bindings 的 Python UniFFI API 验证通用 `acp-v1`。远端为 Linux `6.18.40.1-microsoft-standard-WSL2`、Python `3.13.5`。SSH 与远端进程真实运行，Agent 是仅用于本次验收的合成 ACP v1 peer；没有调用真实 Harness 或模型服务。

本机应用数据和远端执行均使用新建的独立临时目录。OpenSSH 沿用已有登录与严格主机校验配置，不读出认证内容、不改用户配置。peer 的状态、消息及日志均为本次生成的合成数据。

## 有效检查

- 通过 `upsert_acp_runtime` 保存 SSH＋managed HOME 实例，使用 Python 命令和调用者提供的参数启动。包含空格、引号及 shell 元字符的参数保持原值；自定义环境变量及远端 `HOME` 正确传入，peer 的 `SSH_CONNECTION` 为真。
- 本地 resources 目录为空，`remote_resources_directory` 为 `None`，远端没有创建资源安装目录。peer 的 `PATH` 无法发现可执行工具，仍能完成所有交互；本轮只依赖明确指定的 Python 命令，没有 Node 或 Harness 专用 helper 前置。
- `acp_runtime_info` 返回 peer 的 AgentInfo 与协商能力。默认模型使用 `RuntimeDefault`；新建会话、流式消息投影和下一轮发送正常。
- typed 会话配置从 `alpha` 更新为 `beta`，返回值与远端收到的 `session/set_config_option` 一致。
- 标准 `session/request_permission` 产生待答交互。未公布的选项被拒绝，合法选项准确回传后 prompt 完成。取消另一轮 prompt 时，peer 收到 `session/cancel`，应用回到可发送状态。
- 关闭应用再重开后，以原生 session ID 和 cwd 调用 `open_acp_conversation`，远端收到 `session/load`，历史消息回放进入当前会话。该历史只存在于远端合成 peer 的私有目录，本机没有同名远端路径。
- 正常关闭后，本轮启动的远端 peer 全部退出。断连检查只 SIGKILL 本机验收进程直接持有的唯一 SSH 子进程；刻意忽略 stdin 与 SIGTERM 的远端 peer 及孙进程仍被监督层清理，应用报告运行时失败。

以上检查全部通过，没有发现需要产品源码修复的接线问题。远端自建进程和唯一临时目录已清理，并确认目录不存在；本机临时应用目录与 target 下的一次性验收脚本也已清理。未新增自动化测试。

## 证据边界

这证明通用 ACP 路径在真实 SSH 上贯通配置、协议交互和生命周期，不等于真实远端 Agent 的完整兼容性验收。未覆盖真实登录、模型服务或 app 前端；真实 Agent 的独立协议证据见 [原生 peer 验收](native-peer-evidence.md)。SSH bootstrap 仍要求目标具备 POSIX 与 Python 3；主动 daemonize 并脱离监督进程组的外部工具不在断连清理保证内。

正常关闭补修后的适用性复核：本次远端 peer 只声明 `loadSession=true` 与 `sessionCapabilities.list={}`，未声明 close／resume／delete。上述远端证据覆盖无 close 能力的关闭路径；新增 `session/close` 收尾分支由本地六种 peer 场景与真实 DSH 的 shutdown 后再发现验收覆盖，不把本次 WSL 结果扩大为该分支的远端验证。
