# 指定远端主机的 SSH 验收

2026-10-10，用户明确允许使用 `sfp7` 或 `wsl.win-ws.localhost`。本次选择可用的 `wsl.win-ws.localhost`，通过当前 Rust bindings 的 Python UniFFI API 执行隔离验收。SSH、远端操作系统、进程和反向隧道均为真实运行；Claude Code 控制协议使用合成 peer，模型上游使用本机合成 Anthropic Messages 服务。本记录不替代真实远端 Harness 或模型服务验收。

## 环境与范围

| 项目 | 观察结果 |
| --- | --- |
| `wsl.win-ws.localhost` | 现有 OpenSSH 配置下可连接；启用 BatchMode、严格主机校验，关闭 agent／X11 forwarding。 |
| 远端系统 | Linux `6.18.40.1-microsoft-standard-WSL2`。 |
| Python | `3.13.5`，用于远端 bootstrap、合成 peer 和历史 helper。 |
| Node | `20.19.2`；低于当前 Pi／DSH 接入要求，本轮 Claude 路径不依赖 Node。 |
| 真实 Harness 发现 | 非交互 SSH 的 PATH 未发现 Codex／Claude 命令；未继续搜索用户安装、读取用户配置或安装软件。 |
| `sfp7` | 初始 SSH 连接被关闭；没有继续排查或重复执行验收。 |

远端所有写入位于新建的唯一 `<remote-temp>`，包括项目目录、合成 peer、辅助资源及按实例分配的 `<managed-home>`。本机使用另一组独立临时应用目录。沿用 OpenSSH 自己管理的登录和主机校验，不读出凭据，不修改用户 SSH 配置或既有 Harness 数据。

## 已验证结果

- UniFFI 的 `upsert_runtime_with_execution` 可以直接保存无需占位 `agentDir` 的 SSH＋managed HOME 实例。远端先创建私有目录，再安装无凭据辅助资源；远端根路径在本机不存在，运行过程中也没有在本机出现同名目录。
- 版本查询、初始化模型目录和三个正常 turn 均在远端执行。合成 peer 的 `SSH_CONNECTION` 观测为真；三个 turn 的有效模型依次为原生模型、managed 模型、原生模型，managed provider 标记依次为关闭、开启、关闭。两种模型标识均为本次合成数据。
- `HOME` 指向远端按实例分配的 managed 目录，`CLAUDE_CONFIG_DIR` 指向其 `.claude` 子目录。原生 provider 的合成 settings 在 managed turn 前后保持逐字节一致。
- managed peer 实际向远端 loopback 网关地址发送流式 Messages 请求，经真实 OpenSSH 反向隧道抵达本机 Velune Gateway，再由 gateway 调用合成上游。上游收到预期模型 ID，peer 读取并检查了返回的 SSE 内容。原生 turn 没有调用该 managed 上游。
- 关闭应用再通过同一应用配置重开后，历史读取获得三个 turn 的合成消息。JSONL 历史只存在远端 managed HOME，证明历史查询与内容读取没有落回本机；Claude 历史执行仍使用 Rust 内嵌 helper，无需把 Python 源文件投送到远端资源目录。
- 正常关闭后，三个正常 turn 的远端进程均已退出，先前分配的远端网关端口不再接受连接。
- 断连场景使用另一轮合成执行。peer 与其孙进程刻意忽略 stdin 和 SIGTERM。只对本机验收脚本直接持有的唯一运行时 SSH 子进程发送 SIGKILL 后，远端监督进程清除了这两个任务进程；应用报告运行时失败，没有把断连当作成功完成。未使用进程名批量终止，也未向共享 shell 或其它任务的进程组发送信号。

验收结束时，所有本次创建的远端进程与唯一临时目录已清理，并再次检查目录不存在。本机临时应用目录、合成上游和一次性断言脚本也已清理；未新增自动化测试。

## 未覆盖的行为

本轮没有调用真实远端 Claude、Codex、Pi 或 DSH，没有验证远端真实登录、订阅认证、真实 provider 请求、权限交互或 app 前端。真实本机 Harness 的独立证据见 [任务记录](packet.md#已取得的证据)。SSH 监督的清理保证覆盖其创建的进程组；外部工具主动 daemonize 并脱离该进程组不在本次证据范围。
