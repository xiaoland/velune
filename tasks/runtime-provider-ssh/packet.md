# Claude Code、原生模型与 SSH 运行时

2026-10-10 开始。用户授权在 Rust packages 实现 Claude Code、既有 provider 模型选择、每 turn 临时 Velune Gateway、SSH 远程运行时与 Velune 管理 HOME。产品权威归 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，跨单元契约归 [架构](../../docs/design/architecture.md#原生模型单轮网关与-ssh-执行)，调用与环境要求归 [开发说明](../../docs/development.md#rust-运行时的模型来源ssh-与-home)。前端需求明确标为待实现。

## 边界与状态

仅修改共享 packages、独立人工验收脚本及文档。本轮不修改、构建或安装 app，不触碰开始时已有的 Swift／Package／脚本改动。不读取真实配置、凭据或会话，不执行真实登录／模型调用，不 push／PR。用户随后明确允许使用 `sfp7` 或 `wsl.win-ws.localhost` 做 SSH 验收，远端也使用独立临时目录、HOME 与合成上游；其它远端主机未授权。可以自主提交本任务改动，保留已有无关暂存内容。

Claude、原生／managed 选择、SSH 和 HOME 的核心实现、首发前选择切换及跨 Harness 重开身份修复均已完成，workspace 静态检查和构建通过。用户指定 WSL 主机的真实 SSH 传输验收也已通过，实现与验收范围已完成；本地提交仅纳入本任务改动，不发布远端。前端待实现事项不属于本轮源码义务。

不新增或运行自动化测试。验收采用静态检查、构建及显式运行的临时目录／合成上游脚本；实际 CLI 只访问合成 loopback 服务。真实账号、模型服务及平台界面由用户后续验收；指定远端主机的合成 SSH 验收由开发方完成。

## 责任与工程决定

- runtime_adapter 拥有 agent-runtime 的 Claude／模型／传输／HOME；其 SSH owner 完成传输和真实隔离 sshd 验收。
- runtime_application 拥有 application、bindings 与人工用例脚本。
- runtime_design advisor 调查关键生命周期和 SSH 边界；claude_probe 调查实际 CLI 并验证 application 闭环。
- root 维护权威文档、采用 owner 证据、完成集成检查与范围隔离提交。

模型选择区分原生 provider／model 与 managed model record key，下一轮意图不等于正在执行的 provider。managed override 由本轮进程拥有，终态、失败或取消完成时释放；下一轮 native 从既有配置恢复。不为“切回”写回启动前配置快照，不覆盖并发设置变更。Pi SDK 的 RPC set_model 不请求 persist，原生模型变化仍如实进入 session 历史。

SSH 覆盖版本、模型、执行、历史、路径检查与必要写入，远端路径不落回本机。远端要求 POSIX／Python 3；stdin bootstrap 监督独立进程组并在断连时清理。本轮 gateway 使用动态反向转发并验证远端 listener 仅为 loopback。Linux 使用 proc 网络表，macOS／BSD 使用 lsof；无法证明时返回错误。HOME 与 Harness agentDir 分开，受管目录按实例稳定保留。

Pi 提供商导入与 OAuth 来源委托仍只接受默认本机 execution；SSH／existing HOME／managed HOME 可以直接执行原生 provider，但不交给本机认证 helper。跨机器接续沿用来源 cwd，目标须有同一路径，不隐式映射。

固定 Codex 0.159.3 的 thread/start 没有指定 ID 或延迟写盘选项；空线程在同进程 thread/resume 也因无 rollout 失败。metadata read、unsubscribe 均不能迫使合法持久化，空 inject_items 被拒绝。不能以假消息或改标题造历史。采用显式未发送生命周期证据，只在必要时重建 Codex／Claude／Pi 空原生目标，并复用单段 ConversationLink 保持公开身份；不保留虚假来源段。首次 prompt 尝试后不再享有草稿重建资格。

DSH 0.2.0-rc.2 的 resume 无模型覆盖入口，历史 requestHeader 优先于部署缺省；恢复要求旧 provider／model 可识别。set_config_option 只改内存，不能宣称它恢复了持久默认。app 私有 overlay 因此保留已使用网关模型的无凭据 metadata；native 轮次将其地址设为 loopback 端口 0，过滤注入条目并要求选择既有模型后发送。原提供商配置保留，历史不改写。原生选择必须包含 provider ID，严格匹配两项身份。首次模型发现由独立 ACP 进程将 session／storage／query 的持久化定位到 app 私有临时目录或内存，读取 configOptions 后关闭并清理，不在原历史里留下空 session。

Claude 2.1.286 开启 SDK session-state 语义，result 后等待本轮 idle，初始化 idle 不能完成新 turn。长期后台工作可能延迟 idle；按 SDK 区分 local_agent／local_workflow 与其它后台任务，在无待答交互和前两类任务时采用十分钟无主线程活动上限，超时为失败，不伪造成功。官方来源及 CLI 限制见 [Claude 证据](claude-evidence.md) 和 [agent-runtime README](../../packages/agent-runtime/README.md)。

## 已取得的证据

1. 实际 Claude Code 2.1.286 经 application／UniFFI 完成同一原生 session 的 native → managed → native。两个独立 Messages loopback 上游收到预期轮次及认证头，历史正文保留，合成 settings 字节不变。CLI 字段、单次 no-persistence 与持久 resume 实验的范围分别记录在 [Claude 证据](claude-evidence.md)。真实权限弹窗和长寿后台任务尚未完整端到端验证。
2. 实际 Codex 0.159.3 同一 thread 完成 native → managed → native，三个 loopback 请求落到对应上游，原 config 字节不变。config/read 配置的自定义模型补入 model/list 目录并去重；未把缺省模型目录等同于全部配置。
3. 正式 `manual-multi-runtime.py` 通过：实际 Codex／DSH 创建、发送、历史、恢复、cwd／原生日期、同模型 ID 跨提供商、DSH 跨协议、版本不符保留活动连接和日志关联，共五次合成上游请求。移除旧脚本“错误日志不得含临时根路径”的过时判据；产品要求保留错误路径诊断，其余 token、正文、模型和 endpoint 禁采集断言继续保留。
4. 正式 `manual-pi-native-loop.py` 通过：实际 Pi 1.0.2 完成工具续写、下一轮、持久历史重开、提供商编辑／认证导入失效、缺省 cwd、草稿标题／删除与启动诊断恢复，共七次 loopback 请求。来源配置保持不变。
5. `manual-runtime-selection.py` 通过 application／UniFFI 的合成 Claude 协议用例：native／managed 选择、结束／失败／取消清理、下一轮意图保留、跨实例接续、无需 agentDir 的原子 managed HOME 新实例保存与 SSH typed config。独立 `manual-runtime-provider-ssh.py` 验证 Claude 当前分支、标题、时间、来源范围和缺失会话。
6. SSH owner 使用真实 OpenSSH、非特权 localhost sshd、临时合成 host／client keys 和独立 known_hosts，验证远端目录／原子写0600／存在检查、HOME／环境移除、stdin 帧与错误诊断、动态 loopback HTTP 隧道与 Drop 释放。SIGKILL 本地 SSH 后，忽略 stdin 的远端父／孙进程均退出。application／UniFFI 的合成 Claude peer 通过实际 SSH 完成 native → managed → native、远端 managed HOME、网关反向隧道、关闭重开与远端历史 helper；SSH_CONNECTION 与远端 wrapper 证明没有落回本机。local／remote resources 均不含内嵌的 Claude Python helper 时仍通过。所有临时密钥、sshd 和探针均清理，没有连接外部主机或执行真实远程 Harness。
7. 实际 Codex 与 Pi 首发前 native → managed、managed → native 和改交其它 Harness 均通过：每例恰好一次 prompt，公开身份稳定，列表／重开／重命名／删除有效，单目标关联无交接通知。Pi 重建保留已设置的原生草稿名称。原 `manual-cross-harness.py` 的四段重开消息身份断言全部通过，修复为 managed Pi 结束时先同步原生身份再关闭进程。
8. 扩展 `manual-runtime-selection.py` 验证创建失败显式重试、关联原子提交失败不发送且保留已创建目标、未知发送结果后重启禁止发送并可撤除不存在目标的关联，以及单目标关联追加第二段后的重开。失败注入使用合成 peer。
9. `manual-runtime-interactions.py` 保留 Codex 审批、私密回答、忙时拒绝、陈旧回复、取消和意外 EOF 断言；新增 Claude 合成工具批准、AskUserQuestion、拒绝及 interrupt，全部通过。EOF 保留部分正文且禁止继续发送。此证据不替代真实 Claude 权限弹窗。
10. 实际 DSH 未创建业务会话时返回两个既有 provider 的同名模型，来源 patch、sessions 和 projection cache 不变，临时目录已清理；合成 session/new 失败也验证清理。None／未知 provider 均拒绝，不按模型同名猜测。
11. 最终 `cargo fmt --all --check`、workspace 全 targets／features check 和严格 clippy、bindings build 全部 exit 0。bindings 的 no-default-features check／clippy 也通过。Python／Node 薄 helper 语法检查及文档链接检查通过，未运行自动化测试。
12. 用户指定 `wsl.win-ws.localhost` 的真实 SSH 验收通过：application／UniFFI 经 SSH 完成远端版本／模型发现、managed HOME、native → managed → native、远端反向隧道到本机 gateway 和合成 upstream、关闭重开后读取远端历史。正常结束释放子进程与隧道；强制中断本机自有 SSH 后，远端忽略 stdin／TERM 的父孙进程均清理，应用进入失败。合成 settings 字节不变，远端临时目录已删除。远端 Harness 为合成 Claude peer，本机真实 CLI 的协议证据单列；详见 [SSH 证据](ssh-evidence.md)。

真实模型、真实登录、远端真实 Harness 与 app 体验未验收。SSH 监督只能清理由其创建的进程组；自行 daemonize 脱组的外部工具不在此保证范围。前端 composer、SSH／HOME 设置与远端 cwd 输入保持待实现。
