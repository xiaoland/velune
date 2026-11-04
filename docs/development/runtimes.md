# 原生运行时与历史读取

[返回开发导航](../development.md)

Velune 不分发 Agent runtime、Pi SDK、Codex CLI 或 Node。各 Harness 由用户安装，运行时实例保存可执行文件的绝对路径，不依赖 Finder 的 PATH；版本类型与兼容区间见 [agent-runtime](../../packages/agent-runtime/README.md#版本与原生控制)。

| 运行时 | 执行机器上需要 |
| --- | --- |
| Pi 1.4 | Pi CLI 与 Node 22.19+ 的绝对路径 |
| Codex 0.159／0.166 | Codex CLI 与 Node 22.19+ 的绝对路径（历史直接调用 app-server） |
| DeepSeek Harness 0.2 | DSH 包的 JavaScript 入口与 Node 22.19+ 的绝对路径 |
| Claude Code 2.1 | Claude CLI；历史读取使用 Python 3 执行 Rust 内嵌的只读 helper |
| 通用 ACP | ACP 程序或 bridge 的绝对路径；协作 MCP 的 stdio 回退需要 Python 3 |

[`install-runtime-support.sh`](../../scripts/install-runtime-support.sh) 按 [`package-lock.json`](../../packages/agent-runtime/runtime-support/package-lock.json) 把 huihua 及其依赖安装到忽略的 `target/runtime-support`，禁用 npm lifecycle scripts，不修改全局安装。Mac 构建只分发其 parser 依赖和薄 adapter helpers，并带完整许可声明与依赖锁，清单见[第三方声明](../../packages/agent-runtime/runtime-support/THIRD_PARTY_NOTICES.md)。Pi 验收使用执行机器上用户安装的外部 CLI 绝对路径与合成 HOME；仓库不再安装另一份 Pi。

SSH 运行时依赖本机 OpenSSH，以及远端 POSIX 环境、Python 3 和匹配版本的 Harness；采用非交互认证与严格主机校验，应由用户预先完成认证和主机信任。Windows 宿主暂不支持 SSH 运行时（依赖 OpenSSH ControlMaster 与 Unix socket）。执行位置与 HOME 的契约见 [agent-runtime](../../packages/agent-runtime/README.md#原生提供商远端与-home)。
