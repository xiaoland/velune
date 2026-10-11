# 原生资源管理：固定版本与隔离证据

2026-10-11 复核。此文记录 `management` 的入口依据和动态验收，不定义另一套产品契约；公开范围见 [agent-runtime README](../../packages/agent-runtime/README.md#原生资源管理)。所有写入发生在临时 HOME、agentDir 与项目目录，内容和 MCP 地址为合成值。Claude 的模型请求只发送给本机 loopback 替身；没有读取真实认证／会话或调用真实模型 API。

## 原生入口依据

| 版本 | 核对来源与判据 |
| --- | --- |
| Codex 0.159.3 | 固定 tag `rust-v0.159.3` 的 `codex-rs/codex-home/src/instructions/mod.rs` 明确 User 目录先选 `AGENTS.override.md`，再选 `AGENTS.md`。`codex-rs/agent-roles/src/discovery.rs` 递归发现 `.toml`；`loader.rs` 从启用的配置层目录下的 `agents` 加载；`agent_role_config.rs` 给出 `name`、`description`、`developer_instructions`。管理只覆盖 README 所列原生目录，不编辑任意外部 `config_file`。 |
| Claude Code 2.1.286 | 固定安装 CLI 的隔离 `mcp add-json --scope user` 确认显式 `CLAUDE_CONFIG_DIR` 下使用 `.claude.json`；该 CLI 命令仅用于实验，产品直接修改目标 section，不执行其备份逻辑。指令使用 `CLAUDE.md`、项目 `.claude/CLAUDE.md`、`CLAUDE.local.md`；本轮未纳入没有固定版证据的 `AGENTS.md` 候选。技能和 Agent 声明采用 `skills/<name>/SKILL.md` 与 `agents/*.md`。 |
| Pi 1.0.2 | 固定 `@earendil-works/pi-coding-agent` 的 `dist/core/resource-loader.js`：每个目录指令候选依次为 `AGENTS.override.md`、`AGENTS.md`、`AGENTS.MD`、`CLAUDE.md`、`CLAUDE.MD`。`dist/core/skills.js` 明确 User `agentDir/skills` 与 Project `.pi/skills`；根 `.md` 必须有有效 description 才作为 skill，否则是可忽略的普通文件。`dist/extensions/mcp/config.js` 对 user/project `mcp.json` 使用 `JSON.parse`。README 说明没有内置 sub-agent 定义，仅扩展例子提供自有行为。 |
| DSH 0.2.0-rc.2 | 固定安装依赖 `@deepseek-ai/dsh-agent-instructions/lib/index.js` 的 `loadBaselineInstructions`：User 为 `DSH_HOME/AGENTS.md`，项目候选为 `AGENTS.md`、`CLAUDE.md`，本地覆盖为 `AGENTS.local.md`、`CLAUDE.local.md`。`@deepseek-ai/dsh-skill-filesystem/lib/index.js` 的 `FileSystemSkillProvider` 读取 `DSH_HOME/skills` 与项目 `.dsh/skills`；直接 `.md` 和一层 bundle 的 `SKILL.md` 是声明候选，缺 frontmatter 会告警忽略。Cordis MCP／persona／preset 的 patch、插件和注册表语义没有被当成通用 JSON／Markdown CRUD。 |
| 通用 ACP v1 | 协议没有这四类原生持久资源的读写能力；任意启动命令不足以推断 native root。管理返回 Unsupported，不创建猜测的目录。 |

可定位公开来源：[Codex User 指令](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/codex-home/src/instructions/mod.rs)、[Codex Agent 发现](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/agent-roles/src/discovery.rs)、[Codex Agent 配置](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/agent-roles/src/agent_role_config.rs)、[Claude memory](https://code.claude.com/docs/en/memory)、[Claude skills](https://code.claude.com/docs/en/skills)、[Claude subagents](https://code.claude.com/docs/en/sub-agents)、[Claude MCP](https://code.claude.com/docs/en/mcp)、[Pi 官方仓库](https://github.com/earendil-works/pi)、[DSH 官方仓库](https://github.com/deepseek-ai/deepseek-harness)。实时文档和主分支可能继续变化；实施入口以此表固定包和隔离 CLI 观察为限。

Codex 的实际 `skills/list` 隔离实验同时发现 `CODEX_HOME/skills` 与 `HOME/.agents/skills`。本轮 API 只管理前者作为 User root；同样不把 Pi、DSH 的其它共享／自定义 skills roots 冒充为已覆盖。目录是来源目录而非最终有效集合，原生 trust、覆盖顺序和自定义发现配置仍会决定是否加载。

Claude User 与 Project MCP 另用 JSON、JSONC、JSON5 三份隔离输入判别：`mcp get` 只识别严格 JSON 版本，另两份均失败；Pi 固定实现直接调用 `JSON.parse`。因此本轮严格 JSON 解析没有把上述固定版本允许的 JSONC 误判为无效。产品对语法错误只报告无正文诊断，不输出可能含 env／header 的解析器原始行。

## 原生加载与资源语义

通过公共 Rust `management` API 的临时手工 harness 创建 User／Project 指令、skill 声明、二进制附件和可执行附件。观察到读取字节一致、未指定 executable 时保留执行位、旧 revision 返回 Conflict，首次 catalog 不创建缺失的 agentDir。Codex skills 根下普通 `README.md` 能以无 frontmatter 的普通资产写入。DSH native MCP catalog 返回明确 Unsupported。

同一隔离输入随后交给实际 Codex 0.159.3 `app-server`：`skills/list` 返回 API 写入的 skill；`thread/start` 在无模型 turn 下成功。另将 API 写入的合法 Agent TOML 复制为同名声明，原生返回 duplicate agent role 警告；没有复制时不告警。这一差异确认 loader 实际发现了该定义，而非仅证明管理 API 能读自己的输出。没有执行子 Agent 或模型请求。

实际 DSH rc.2 的公开 `loadBaselineInstructions` 和 `FileSystemSkillProvider` 在显式临时 roots 下加载 API 写入的 User／Project 指令与两份技能，并读取到预期合成 body。provider 未开启 watcher，结束后 dispose。此处验证来源加载，不验证模型执行或 Cordis authoring。

MCP 手工用例在 Codex 共享 `config.toml` 中放入合成 provider 和两个服务：选中服务的 read 不含 provider section，unknown native 字段保持；修改保留非目标注释和 sibling 服务。新增其它服务后旧 revision 失效；删除只影响目标条目。坏 TOML 产生 Invalid 目录诊断，后续 Missing 写入不覆盖它；Debug 与错误不含合成秘密正文。JSON 修改保留其它键的语义，但不承诺排版保真。

Claude／Pi 的可复现实际加载入口为 [manual-runtime-management.py](../../scripts/manual-runtime-management.py)；application／UniFFI 的独立 API 边界入口为 [manual-runtime-management-api.py](../../scripts/manual-runtime-management-api.py)。后者覆盖配置和生命周期，不能替代原生 loader 证据。它们是手动入口，不接入测试或 CI。

最终 Claude 2.1.286 验收通过 UniFFI 写入唯一名称的 User instruction、skill、Agent 定义和 MCP 服务。`stream-json initialize` 的 `agents` 列表出现所写 Agent 名称；`mcp list` 发现所写服务。随后用 `/velune-resource-skill` 启动一轮指向 loopback Messages 替身的请求，请求内同时包含 API 写入的 `CLAUDE.md` 标记和 skill 正文标记，CLI 接收替身的 `RESOURCE_ACK` 终态。没有同名项目资源遮蔽，也没有在 API 写入后用脚本覆写 fixture。Claude 初始化会更改共享配置中的其它字段；删除合成 MCP 服务前须重读 revision，旧 revision 会正确报冲突。

Pi 1.0.2 的 `DefaultResourceLoader` 直接读取 API 写入的 User／Project 指令和 `.pi/skills`／`agentDir/skills`；`getAgentsFiles()` 和 `getSkills()` 分别返回唯一标记。验证使用独立安装目录中的固定公开包及 Node 24.15.0，不复制用户配置。原生子进程均处于 macOS sandbox：拒绝 `/Users`、系统 Managed Preferences 和 ConfigurationProfiles 的读取，只对指定的运行时可执行文件放行；禁止非 loopback 网络。Node 目录探针已确认 `/Users` 被拒绝。原生调用有超时，应用、临时配置和替身服务均在 finally 中清理。

复现先按 [bindings 说明](../../packages/bindings/README.md#生成绑定)构建并生成匹配的 Python 绑定，再执行以下手动命令。`PI_MODULE` 指向独立安装在 `/Users` 之外、包含完整依赖的 Pi 1.0.2 `dist/core/resource-loader.js`；其余变量为本次库、绑定目录和已安装的 CLI／Node 路径。脚本会校验固定版本，不允许取消 sandbox。

```sh
python3 scripts/manual-runtime-management-api.py --library "$LIBRARY" --bindings "$BINDINGS"
python3 scripts/manual-runtime-management.py --library "$LIBRARY" --bindings "$BINDINGS" \
  --claude "$CLAUDE" --node "$NODE" --pi-module "$PI_MODULE"
```

## 资源库写入的加载

2026-10-11，[manual-resource-library-native.py](../../scripts/manual-resource-library-native.py) 以同样的 sandbox 约束（拒绝读取 `/Users`、Managed Preferences 与 ConfigurationProfiles，只放行 Node 可执行文件；仅 loopback 网络；临时 HOME 与 agentDir）通过 UniFFI 保存资源并启用到停用的 Claude Code 2.1.286、Codex 0.159.3 与 Pi 1.0.2 实例：指令、技能、stdio MCP（工作目录 `/tmp`、超时 30 秒）、HTTP MCP（请求头）与子 Agent（Claude、Codex）。写入后：

- Claude：`mcp list` 列出两个服务，`mcp get` 读出 stdio 命令（严格 JSON 解析通过，写入 `timeout` 30000、无 `cwd`）；stream-json `initialize` 的 `agents` 含所写子 Agent，`commands` 含所写技能。
- Codex：`mcp list --json` 读出 stdio 的 `cwd`、`tool_timeout_sec = 30` 与 HTTP 的 `url`；`app-server` 的 `skills/list` 返回所写技能。子 Agent TOML 与上文已由原生 loader 发现的格式相同（`name`／`description`／`developer_instructions`），本轮未另做发现实验。
- Pi：固定包自身的 `loadMcpConfig` 无错误地接受两个服务，`cwd` 与 `timeout`（秒）保留。Pi 依赖以 `npm install --omit=dev --ignore-scripts` 装在 `/Users` 之外的临时目录。

指令与技能正文的实际进入请求已由上文 Claude／Pi 实验覆盖，资源库写入的是同样的原生文件；本轮不调用模型，也不发起 loopback 模型请求。

```sh
python3 scripts/manual-resource-library-native.py --library "$LIBRARY" --bindings "$BINDINGS" \
  --claude "$CLAUDE" --codex "$CODEX" --node "$NODE" --pi-mcp-config "$PI_MCP_CONFIG"
```

## 文件边界

资源操作依赖目标端目录句柄和 no-follow 打开。本机 macOS 与用户授权的 WSL 临时目录执行相同公共 API fixture；远端使用 `ExecutionTarget::Ssh` 实际传输，不以本机文件系统模拟远端。

已观测深度超过 64 时，本地／SSH 的 catalog 和直接路径操作均返回 Invalid。8 个并发调用以 Missing 创建相同路径，两端都恰有一个成功、其余 Conflict，最终内容属于胜出调用；这验证首次创建独占发布，不将已有文件的乐观 revision 更新夸大成外部编辑器事务锁。临时资源根在两端清理。

完整矩阵还通过：二进制 CRUD；新普通文件 0600、可执行文件 0700、目录 0700；既有 0751 mode 保留，取消 execute 仅清除 0111；内容或 mode 变化使旧 revision 冲突；文件与空目录可删除、非空目录拒绝；root／内部 symlink 不跟随且可见项只读，外部祖先 symlink 可解析；FIFO、16 MiB + 1 文件、只读父目录、`..`／绝对路径／NUL、4097 节点都得到明确分类。macOS 与 WSL Linux／Python 3.13.5 对相同内容和 mode 产生相同 SHA-256 revision。Python 目录枚举使用有界流式迭代，不先分配全目录名称列表。最后 helper 语法检查与包级 clippy 通过，文件 owner 的临时程序、构建物和两端数据全部删除。本机其它已编译支持的 POSIX 平台未在本轮逐个运行；Windows 本机资源操作明确 Unsupported，Windows 上的 WSL SSH 是另一种执行目标。
