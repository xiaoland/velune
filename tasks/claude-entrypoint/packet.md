# Claude Code 入口标识与桥接缓存

2026-11-02。用户选择先验证 `CLAUDE_CODE_ENTRYPOINT=velune`，保留现有 stream-json 与控制能力，再研究订阅桥接的缓存连续性。

当时范围：隔离 HOME、合成凭据与回环上游下，用已安装的真实 Claude Code 对照默认入口与 velune；验证消息流、MCP、审批、中断、恢复及侧聊等现有能力。验证通过后在两条执行路径显式使用自有入口。缓存部分先返回有证据的改进方案，不擅自增加持久化内部会话。无真实账户／计费联调授权。用户已批准提交本任务改动，其他任务的未提交修改不包含在内。

当时判断条件：请求和会话中的入口确为 velune；stream-json 与所检查的控制能力保持有效；区分协议机制验证与尚未验证的真实五小时配额收益。缓存方案要解释进程复用、重建、上下文变化及未完成工具结果的处理。

当时实现：原生 adapter 与订阅 backend 都走 print；前者每轮 resume，后者按 binding/session 复用最多 64 个进程，空闲 10 分钟回收，随机 cwd、无会话持久化。上游对配额差异的报告仍属社区证据。

## 已取得的证据

- 已安装 CLI 为 2.1.292。真实 CLI、隔离 HOME／CLAUDE_CONFIG_DIR、合成 API key、回环 Messages 与 MCP 上游：默认请求的 billing header 为 `cc_entrypoint=sdk-cli`，设置后全部为 `velune`。10 个合成模型请求覆盖 initialize 模型目录、stream_event、动态 MCP 接入与一次真实工具回调、Write 审批拒绝且未写文件、side_question、中断后继续发送，以及跨进程 resume 保留历史，全部通过。
- 原生执行、重命名 helper、订阅执行子进程已显式设置 `CLAUDE_CODE_ENTRYPOINT=velune`；未改变参数及控制协议。两份 unit README 记录当前验证版本与配额收益边界。
- `cargo check --locked -p velune-agent-runtime -p velune-ai-provider --all-targets` 通过。修改的两个 Rust 文件通过独立 rustfmt 检查；`git diff --check` 通过。相对 Markdown 链接检查通过，新增链接无标题锚点。全工作区 fmt 检查发现任务开始前已存在的其他文件格式差异，未扩大修改范围。
- 额外的实际 adapter Rust 探针未取得运行证据：首轮在编译／共享构建目录等待期间达到 600 秒上限；重跑仍排队，已结束本任务拥有的进程树。Clippy 同样因共享目录构建锁排队而主动结束，不能计为通过。CLI 协议验证与相关包编译足以支持此次仅设置环境变量的修改；不把未运行的 adapter 探针当作端到端验收。临时 examples 已从仓库删除，源码副本留在实验目录。未停止其他任务的构建。
- 实验文件和合成证据位于 `/var/folders/rk/8_krr0y14p9g5plk8n77lgyr0000gn/T/velune-claude-entrypoint-eiq6663h`；脚本不提交。任务开始时四个相关文件的副本保存在该目录 `before/`，用于区分其他未提交修改。

## 2026-11-27 订阅桥接缓存连续性

用户授权在独立 worktree 和分支实施。订阅桥接在应用私有目录使用稳定 cwd，按外层会话、提供商、账号与认证代际保存最近一次确认完成的原生 CLI 会话。在配置和调用方历史前缀精确匹配时，重启后通过原生 `--resume` 只提交新用户消息。未完成的工具请求仍属于原进程。无法恢复时在提交用户输入前改用完整调用方历史，CLI 不接受私有会话文件时改用无持久化启动。详细契约归 [AI Provider](../../packages/ai-provider/README.md)。

以 Claude Code 2.1.327 和回环 Messages 上游运行真实订阅桥接。普通文本续聊及完成 MCP 工具调用后的续聊，比较持续运行与关闭并重建进程池两条路径。两组第二轮请求的历史消息、工具定义和可缓存系统提示相同；合成调用没有向真实 Anthropic 发送请求。过期服务端缓存、真实订阅命中率与额度收益不在该证据范围内。

回归覆盖同账号精确前缀恢复、未完成轮次丢弃、历史改写、换账号和 CLI 拒绝私有恢复时的回退。任务分支为 `feat/claude-bridge-cache`。
