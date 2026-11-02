# Claude Code 入口标识与桥接缓存

2026-11-02。用户选择先验证 `CLAUDE_CODE_ENTRYPOINT=velune`，保留现有 stream-json 与控制能力，再研究订阅桥接的缓存连续性。

当前范围：隔离 HOME、合成凭据与回环上游下，用已安装的真实 Claude Code 对照默认入口与 velune；验证消息流、MCP、审批、中断、恢复及侧聊等现有能力。验证通过后在两条执行路径显式使用自有入口。缓存部分先返回有证据的改进方案，不擅自增加持久化内部会话。无真实账户／计费联调授权。用户已批准提交本任务改动，其他任务的未提交修改不包含在内。

判断条件：请求和会话中的入口确为 velune；stream-json 与所检查的控制能力保持有效；区分协议机制验证与尚未验证的真实五小时配额收益。缓存方案要解释进程复用、重建、上下文变化及未完成工具结果的处理。

已有事实：原生 adapter 与订阅 backend 都走 print；前者每轮 resume，后者按 binding/session 复用最多 64 个进程，空闲 10 分钟回收，随机 cwd、无会话持久化。上游对配额差异的报告仍属社区证据。

## 已取得的证据

- 已安装 CLI 为 2.1.292。真实 CLI、隔离 HOME／CLAUDE_CONFIG_DIR、合成 API key、回环 Messages 与 MCP 上游：默认请求的 billing header 为 `cc_entrypoint=sdk-cli`，设置后全部为 `velune`。10 个合成模型请求覆盖 initialize 模型目录、stream_event、动态 MCP 接入与一次真实工具回调、Write 审批拒绝且未写文件、side_question、中断后继续发送，以及跨进程 resume 保留历史，全部通过。
- 原生执行、重命名 helper、订阅执行子进程已显式设置 `CLAUDE_CODE_ENTRYPOINT=velune`；未改变参数及控制协议。两份 unit README 记录当前验证版本与配额收益边界。
- `cargo check --locked -p velune-agent-runtime -p velune-ai-provider --all-targets` 通过。修改的两个 Rust 文件通过独立 rustfmt 检查；`git diff --check` 通过。相对 Markdown 链接检查通过，新增链接无标题锚点。全工作区 fmt 检查发现任务开始前已存在的其他文件格式差异，未扩大修改范围。
- 额外的实际 adapter Rust 探针未取得运行证据：首轮在编译／共享构建目录等待期间达到 600 秒上限；重跑仍排队，已结束本任务拥有的进程树。Clippy 同样因共享目录构建锁排队而主动结束，不能计为通过。CLI 协议验证与相关包编译足以支持此次仅设置环境变量的修改；不把未运行的 adapter 探针当作端到端验收。临时 examples 已从仓库删除，源码副本留在实验目录。未停止其他任务的构建。
- 实验文件和合成证据位于 `/var/folders/rk/8_krr0y14p9g5plk8n77lgyr0000gn/T/velune-claude-entrypoint-eiq6663h`；脚本不提交。任务开始时四个相关文件的副本保存在该目录 `before/`，用于区分其他未提交修改。

## 缓存判断与改进候选

观察：进程存活且调用方历史严格追加、执行 signature 不变时，现有后端只发送新增消息，已经保留内层对话。空闲回收、应用退出或 signature／历史变化后重新启动，使用新的随机 cwd，并把调用方完整历史包装成一条用户 JSON 文本。进程退出不等于服务端缓存消失；这里真正改变的是缓存前缀及消息形状。

建议第一步：由 application 提供稳定的私有工作目录，经 gateway 显式传入 ai-provider；临时 MCP／system 文件仍放各 run 的临时目录。目录不按外层会话或认证世代变化，保留 native tools／hooks／来源设置的禁用边界。这能稳定环境提示及公共前缀，不宣称恢复全部历史缓存。

完整冷启动连续性有两条路线，尚未选定或实施：

1. 保持无内层历史持久化，把冷启动历史呈现为稳定分块并有界设置缓存 checkpoint。真实 2.1.292 的合成探针确认 user text block 的 `cache_control` 可透传；两次同 cwd 新会话的稳定块均保留 `ttl=1h`，每次总计 4 个标记。正式方案仍需处理调用方工具、Claude 自己的缓存槽位、20-block 回溯和 warm/cold 两种消息形状，不能由这条透传证据推断真实缓存命中。
2. 持久化外层 session → 内层 Claude session 的绑定，稳定 cwd，按精确已完成 checkpoint 与 signature 校验后 resume；完整提交后 clean、执行前 dirty，中断或工具结果未完成时不冒险恢复。这会新增 Harness 内部会话、清理和会话可见性责任，代价明显更高。

当前返回：入口标识修改完成，CLI 机制验证与包级编译通过；缓存改进方案待评审，尚未实施。真实订阅额度收益、缓存命中率、额外 adapter 端到端证据与 Clippy 仍未取得。下一步建议先实施稳定私有 cwd，再按实际冷启动频率决定稳定分块或持久化内层会话的路线；真实账户实测需另有对应授权。
