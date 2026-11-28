# 将 Cage 内的 Claude Code 添加为 Agent 运行时

当前阶段是调研。目标是在 Velune 中浏览并续聊 Cage 内已有的 Claude Code 会话，优先复用已有配置入口。正式添加实例、启用私网以及使用真实订阅和会话续聊尚未进行。

## 结论

单个配置目录中的 Claude Code 会话可以复用 Velune 现有的「Claude Code 2.1」和「SSH」运行位置。SSH 的执行目标须是 Cage 内的 Debian 环境。选「继承当前用户 HOME」，运行时入口填环境内 `command -v claude` 的绝对路径，运行时目录填该会话所属的 `CLAUDE_CONFIG_DIR`（默认 `$HOME/.claude`），另选独立的远端辅助目录。会话继续使用 Cage 内保存的工作目录。[Velune SSH 设置](../../app/mac/Views.swift) · [运行时前提](../../docs/development/runtimes.md)

- Cage 文档提供通过 Tailscale 以 OpenSSH 公钥登录环境内普通用户的方式。本机 Cage 0.2.1 尚未安装或启用 Tailscale。临时试验使用 Cage 已有的本机 Lima 回环 SSH，独立核验了主机公钥。Lima 是 Cage 的内部连接，端口在重启后的稳定性未验证。[Cage 私网 SSH](https://claude-cage.com/docs/tailscale/)
- `cage run` 可以启动 Claude，但本机运行时会在 macOS 上读取会话目录；仅把入口换成启动脚本无法统一启动、历史和工作目录的执行位置。[Cage 命令行](https://claude-cage.com/docs/workbench/) · [Velune 历史读取](../../packages/agent-runtime/src/history.rs)
- Cage 的账号调度、跨账号接续和账号专属出口由其 Claude 终端启动流程管理。SSH 直启 Claude 的试验只覆盖单个配置目录，不能据此确认这些能力。Velune 当前通过本机文件与进程观察其他客户端的活动，SSH 实例不会报告 Cage 中已运行的终端会话。续聊前应先确认原会话已空闲并退出原终端。[Cage 多账号](https://claude-cage.com/docs/accounts/) · [Velune 外部运行状态](../../packages/application/src/local/live.rs)

## 验证

2026-11-27，在本机 Cage 0.2.1 中确认 Claude Code 2.1.332 和 Python 3.13.5。使用临时 HOME、单独的 `CLAUDE_CONFIG_DIR`、合成 API key 与客机回环模型响应，通过 `cage run` 创建原生会话。随后经非交互 OpenSSH 使用 Velune 现有的 `remote_bootstrap.py` 和 Claude 历史脚本完成索引、摘要和打开历史。重新以 `stream-json` 初始化并按同一 ID 执行 `--resume`，回环服务确认第二轮请求包含首轮问答，历史脚本也读回两轮。临时目录和探针进程已清理。

正式接入待验证的是 Mac 运行时表单保存、配置账户及原会话可见性。先确认采用本机回环连接还是 Cage 支持的私网 SSH，再在 Velune 添加实例并打开一条原会话。真实续聊还须核对原账号和项目目录。
