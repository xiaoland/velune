# 首轮审计记录

以下记录截至首轮交付时的源码与回执。重新审查的逐声明判断、已退役组件和最终执行结果见[重审报告](reaudit.md)。

合法的 IPv6 上游现在可以使用，Pi 暂时写满时能继续派发控制命令，Linux 资源操作在文件发布后能正确同步目录，Claude Code 2.1.327 可以沿用原身份恢复空导入。验收现在能通过生成绑定和 Mac 新进程重开真实状态，检查正文与附件内容，并把结论关联到实际使用的程序和资源。

初始基线为 `2d37a98c7`，独立工作树为 `.worktrees/0680/velune`，分支为 `feat/test-contract-audit`。用户随后授权推送 develop。第一次同步到 `ccc214a73` 后，补审了新增的两个 Claude 桥接缓存用例。交付前再次同步到 `d6e1e7a9c`，保留基线新加的侧聊回复与状态轮询回归。此前取消任务的现场及其它工作树未被覆盖或清理。

## 审计范围与方法

全文复核初始基线的 822 个 Rust 测试声明及参数表、相关实现／调用方／历史／CI，另覆盖随仓库脚本、39 个 Mac 检查文件、既有五个 Python 验收用例和生成绑定入口。按运行时、应用与绑定、服务与存储、平台工具、跨语言验收划分责任。每个声明记录保留、修复、合并或删除及理由。详细工作账本保存在本工作树 `tmp/test-contract-audit/*-ledger.md`。下表保留交付所需的判断和剩余保护。

先用 test-hygiene 逐项判断断言有效性、重复成本与保留理由，再用 test-adequacy 从行为倒查缺失证据和执行边界。两方面指导取自 abyss 的公开提交。[prune-tests](https://github.com/caldoai/skills/blob/d5b68dace378219890d821adb3685538e2c69b04/skills/prune-tests/SKILL.md) · [abyss 425a286](https://github.com/BeautyyuYanli/abyss/commit/425a2863549692f21728502eb698086e8c3ad64d)

OpenClaw 的公开整合方法用于分工、逐项盘点、删除前对照及定向破坏验证。范围只含公开材料。其 pnpm、Vitest、行数限额和发行规则属于 OpenClaw 自身流程。[test-audit](https://github.com/openclaw/openclaw/blob/a1ae1c43912c3aea1f6fcd9eb1da0538a8008634/.agents/skills/test-audit/SKILL.md) · [CAMPAIGN](https://github.com/openclaw/openclaw/blob/a1ae1c43912c3aea1f6fcd9eb1da0538a8008634/.agents/skills/test-audit/CAMPAIGN.md) · [审计清单](https://github.com/openclaw/openclaw/blob/a1ae1c43912c3aea1f6fcd9eb1da0538a8008634/.agents/skills/test-audit/references/audit.md) · [证明方针](https://github.com/openclaw/openclaw/blob/a1ae1c43912c3aea1f6fcd9eb1da0538a8008634/.agents/skills/openclaw-testing/SKILL.md#proof-policy) · [编写测试](https://github.com/openclaw/openclaw/blob/a1ae1c43912c3aea1f6fcd9eb1da0538a8008634/docs/help/testing/writing-tests.md) · [场景契约](https://github.com/openclaw/openclaw/blob/a1ae1c43912c3aea1f6fcd9eb1da0538a8008634/docs/help/testing/boundary-scenarios.md) · [夹具](https://github.com/openclaw/openclaw/blob/a1ae1c43912c3aea1f6fcd9eb1da0538a8008634/docs/help/testing/fixtures.md)

对每个删改候选，复核实现入口、真实调用者、引入历史和剩余覆盖。负向夹具从有效输入出发，只改变一个目标条件，观察具体错误。对恢复的关键断言，临时改坏生产实现，核对失败位置与还原后的结果。

## 实际改动与保留的契约

| 边界 | 原有问题与改动 | 剩余证明及边界 |
| --- | --- | --- |
| 公共 HTTP endpoint | `parse` 把带方括号的 IPv6 host 交给结构化地址校验。`new` 接受 IPv6 后输出 URL 又缺方括号。修复只在 endpoint owner，两个公开构造入口使用固定 DNS／IPv4／IPv6 期望。 | 固定地址从公共 API 输入并从最终 URL 输出核对。网关 IPv4 请求及协议转换各有独立覆盖。 |
| Pi 写入背压 | 端口保留八个控制槽，但物理 writer 只保留四个。第五个控制帧遇到暂时满载会使健康连接失败。现在保留尚未交给 writer 的原始帧、顺序与截止时间，待容量恢复时继续派发。 | 真实子进程先消费大 prompt 的一个字节并暂停。解开管道后依次收到 prompt、六个对话回复、clear_queue、abort，共九帧。每个票据只产生一次 Written。 |
| Linux 本机资源落盘 | 发布文件或删除后，从 cap-std 复制出来的 Linux 目录句柄仍带 `O_PATH`，同步时报 `EBADF`。现在相对已持有的目录打开可读句柄并同步。 | 保留原有修订冲突和递归列举／删除测试，均通过实际目录与文件执行。只能验证系统调用与用户可见状态，断电恢复未模拟。 |
| 账户 HTTP | 原测试复制了生产请求／响应处理，名称承诺禁止重定向却只收到 200。删除复制 helper，测试实际生产 client policy 与 body owner。 | 真实回环 HTTP 核对 headers、200、65,536／65,537 字节边界、损坏 JSON、307 和 503。可成功的重定向目标使错误跟随可观察。503 用例验证失败映射。 |
| 配置／远端／遥测格式 | 未来版本输入缺少其它必填字段，可能在目标版本 guard 之前失败。v3 contact 同时有多个格式错误。 | 从完整有效 fixture 只改版本或一个字段，核对准确错误、原字节与备份状态。已知迁移、损坏格式和未知字段仍各有保护。 |
| 持久化恢复 | 部分检查只看 revision、内存状态、文件可读或原 owner。 | ForkRegistry、远端撤销高水位、遥测完整 profile、composer 附件均读取实际写入内容。需要 owner 重开的用例先销毁旧 owner。Mac wrapper 对 ownership／experience 在同一私有目录启动第二进程，并检查附件精确字节。 |
| 运行时投影 | “替换旧通知”从没有旧通知的空投影开始，否定断言无效。 | 先通过实际事件加入旧通知，再以权威快照替换，固定最终消息顺序。只移除完全重复的两行参数。 |
| 实际 CLI／SDK | 缺少可选环境变量即 return，被记为通过。部分用例名称将同 owner reload 称为 reopen。 | 真实依赖检查改成显式 ignored，主动启用却缺条件会失败。合成默认路径仍运行。同一实例重新加载的用例更名为 reload。 |
| ACP／Claude／Pi 夹具 | ACP 把异步 Written 后的回复误判为仍 pending。Claude 假进程缺 command_lifecycle ACK。Pi 已接受请求却发中间 awaiting_user 回执，阻塞调用。 | ACP pending 拒绝移到真正堵塞 stdin 的用例，Written 后核对 stale。假进程遵循当前协议，等待有截止。真实 awaiting_user 与生产准入保护仍独立保留。 |
| 物理写入与回执 | 一个 Pi 检查把一次 poll 当成槽位已经可用，另一个 Claude 检查的 80 ms 期限可能在编码期间耗尽。 | Pi 只在精确的容量满错误下继续，有界消费回执并检查被放弃票据只出现一次。Claude 子进程先从管道读出命令首字节并发布标记，检查同一绝对期限过后返回 timeout、排空回执仍需清理。 |
| Claude 空导入 | 原来只写一条队列标记，实际 CLI 拒绝用相同 ID 新建，直接恢复又找不到可重放记录。现在写原生 `system/turn_duration` 元数据，按原身份恢复。只有精确匹配的旧单条标记才在备份核验后追加兼容记录。 | 两个升级进程并发调用仍只追加一次。已有用户消息、未知或损坏记录、备份冲突各自保留原始字节。实际 CLI 检查经 Velune 的 open 和 initialize 流程，分别重开新导入和旧标记。 |
| 新基线的 Claude 缓存恢复 | 原拒绝用例在假进程启动即退出，只观察最后回退，无法证明恢复实际尝试及初始化拒绝阶段。 | 保留启动即退出行，增加实际 control_request／initialize 拒绝行。按 attempt 记录启动和首帧，核对唯一 stateless 回退收到完整三条历史。回归观测的是子进程实际读到的帧。原跨 pool／文件恢复正例仍保留。 |
| 网关额度准入 | 三协议、两种等待原因、两种 stream 形态串行累加生产五秒等待。 | 每行有独立 Runner／SchedulingState／不可变 quota，行与请求形态并发。全部状态、Retry-After、错误和上游零调用断言保留，生产等待不变。 |
| Mac 负向与可重放性 | 开发实例负向测试不抛错也能通过。布局 fixture 同时违反两个所有权不变量。随机失败缺 seed。目录 UUID 重复被路径重复挡住。 | 有效对照和单点负向、准确拒绝原因。固定 200×60 序列并输出 seed／step。目录身份与路径分别验证。 |
| CI 输入归属 | 仅看 `.rs` 等后缀，嵌入 Python／JS／Markdown／fixture、生成绑定和验收变化可能不执行 Rust 门禁。重命名可能遗漏旧路径。 | 按包／工具／第三方／Cargo／验收 owner 树路由，NUL 分隔且禁用 rename 合并。独立路径表与真实 Git 重命名检查。默认运行 runner 回归，相关变化运行真实生成绑定的合成验收。 |
| 验收结果与输入 | worker 写 passed 后非零退出仍算通过，损坏结果可能没有清单。HEAD 无法说明外部库／helper 来源。 | 父进程同时核对退出码、结果和已通过的检查。旧版将空记录或 `saved=false` 报成 PASS，三个子进程反例在修复前均错误返回 0，修复后六组 unittest 通过。记录动态库、生成模块、用例、helper 与链接依赖内容，复用现有生成回执核对配套。无回执明确标空，不推断编译来源。 |
| 跨语言用户路径 | 缺少默认可执行的配置／未发送输入持久化验收。 | 公开生成绑定保存 provider／合成 key，重开，修改并保留 model identity／key，再保存 Unicode 草稿与二进制附件，第二次重开读取精确内容。不预写 repository。 |

合并 Responses metadata 小测试前，将原有 user、model 断言并入完整请求体和诊断事件测试。投影参数表删去两行相同输入与预期。Mac 侧栏检查删去一个只被自身三条常量断言调用的函数，工具栏分区、控件和几何关系仍通过真实 AppKit 布局检查。账户 HTTP 回归改调生产实现后，删除了仅测试使用的重复请求函数。

因传输、生命周期和平台风险不同，分别保留 Rust 协议与 Node／Python 桥接、网关准入内在状态与 HTTP 错误表达、远端信任政策与 TLS 认证及恢复、迁移备份与当前格式拒绝、Rust 应用与生成绑定／Mac 控件。静态资源装配、慢的持久化／锁／队列压力和有效测试接缝也保留。

交付前，`develop` 已包含 Linux 目录同步、锁类型适配、测试构建的曲线运算优化、Claude／Pi 部分夹具修复。最终 PR 沿用基线的 Pi 接受回执顺序，并保留独立的真实管道背压回归。Claude 的大记录检查由读写双方共同加锁，读取快照中的每条记录都须成功解析。基线新增的侧聊回归通过公开命令创建侧聊、等待延迟回复、检查恢复可发送、游标无新变化及父会话消息不变，保留原样。下文保留本次审计观察到的缺陷及其修复前后证据。

下表以最终 PR 基线 `d6e1e7a9c` 为比较起点，只计改动文件的物理行数，含空行和注释，不含文档。混合 Rust 文件分开统计生产主体与测试专属部分。新增内容主要用于修复证明和实际执行边界，数量用于估算维护成本。

| 职责 | 基线 | 候选 | 变化 |
| --- | ---: | ---: | ---: |
| 产品实现 | 6,828 | 6,988 | +160 |
| 测试／夹具 | 14,749 | 15,838 | +1,089 |
| 验收 runner、CI 与执行支撑 | 720 | 869 | +149 |

## 验证记录

所有重编译通过 agent-build，使用仓库已有 target 域。Askama 在符号链接 Cargo registry 下产生错误相对路径。用同一缓存的 canonical CARGO_HOME 解决，没有修改全局配置或绕过编译锁。

十个原有 PR 合成脚本在基线上通过。验收 runner 修复前真实子进程 exit 7 被误判 PASS，修复后六项 unittest 通过。CI 路由新用例在旧实现出现 13 个目标失败，修后通过。Mac 身份 guard 被破坏时旧检查仍通过、新检查按目标失败。Mac 缺少重开阶段的旧 wrapper 被三项断言拦住，修后四项编排检查通过。

完整 Rust 基线暴露 ACP 时序假设和 Claude 命名夹具失败。五项 Pi 检查在相同 prepare_other 等待超过四分钟，已采样定位并只终止本次测试进程及后代。这五项的基线结果为阻塞。网关 dispatch 的 19 项在基线与候选均通过，墙钟耗时从 60.54 秒降至 5.12 秒。

在同步后的基线上，两个 IPv6 回归修复前分别报合法地址无效和 URL 缺方括号，修复后通过。ACP 定向对照证明 Written 后已失效，单独阻塞管道则能稳定验证 already_pending。应用层整合 293 项通过、2 项按条件忽略，用时 20.04 秒。

11 个 Rust 变异控制在应失败的断言上失败，逐字节还原后各自通过。控制分别覆盖跳过 Claude 恢复和在 initialize 之前发送用户帧、保留旧投影通知、遗漏 ForkRegistry 写盘、跟随账户重定向、接受远端未来版本、接受 v3 的 v4 字段、遗漏远端撤销记录、接受遥测未来版本、遗漏持久化 profile，以及未移除不允许的 metadata。Swift 布局模型移除跨组 tab 唯一性判断时只在该用例失败，还原后全部 36,077 个断言通过。

Pi 控制槽用例在同一个旧二进制上重复 12 次，有 2 次失败。它的旧日志没有打印错误，不能单凭这两次推断物理 writer 满。另加的管道回归在旧产品实现精确观察到容量满导致断线，修复后九帧及票据断言通过。Claude partial-write 的一次旧失败发生于 admission，日志明确是“运行时请求尚未派发”。新夹具把字节已进管道列为硬前提。

单独运行 Claude 空导入检查时，已安装的 2.1.327 在改动前报 `Session ID … is already in use`。同一旧记录直接用 `--resume` 则报找不到会话。六组私有目录和回环上游对照验证了新原生记录只补会话身份。请求中只有本轮用户输入和 CLI 自带的 system 配置。在旧转换记录之前或之后追加另一条用户消息，两种顺序都能保留该消息。将生产 opener 临时恢复为旧逻辑后，同一个 installed 测试再次在 initialize 收到 `Session ID … is already in use`。逐字节还原后通过。Python helper 并发升级、备份发布失败及重试也在合成目录验证。

Linux 的初次全量运行中，两个 `management::fs` 用例执行真实文件写入时返回 `Bad file descriptor (os error 9)`。首次验证还暴露了锁常量的跨平台 Clippy 类型差别，已修复。此次 Linux 同时观察到五个 mesh 路由超时，后续验证按相同路由契约进行。[初次 Linux 运行记录](https://github.com/xiaoland/velune/actions/runs/46098292864)

Mac 开发包的四项后台原生检查通过。ownership 和 experience 各自先成功退出，再由不同 PID 在同一私有目录重开。aggregated-list 核对重复 UUID 和重复路径各自的拒绝原因，sidebar-toolbar 读取 30 个明暗布局中的真实几何数据。回执和二进制摘要位于本工作树 `target/verification/test-contract-audit/<检查名>/receipt.json`。该包基于 `f7ccedfb5` 的平台检查代码。

加入 Claude 空导入兼容修复之前，本机完整 Rust 工作区 37 个目标共通过 818 项，失败 0 项，11 项按条件忽略。耗时 150.31 秒，含增量编译。全工作区 Clippy 和格式检查通过。Linux 修复前写盘失败的两个原用例、Pi 真实管道、Claude partial-write 都在后续 Linux 构建中通过。[Linux 修复验证](https://github.com/xiaoland/velune/actions/runs/46099759760)

同一轮 Linux 的 A–B–C–D 路由中，前两跳取得连接各约 100 微秒，随后的 Sync 却各耗约 1.94 秒。第三跳进入 Sync 后耗尽五秒请求预算。本机晚段采样的 1,501 个非等待样本中，811 个落在曲线运算栈上，来自信任状态验证及合并等路径。因此只在 Cargo test profile 优化 `curve25519-dalek`，保留真实密码学、五秒期限和业务中继断言。该设置下本机 66 个 remote 检查通过，Linux 工作区也已通过此前失败的两个场景。[多跳修复验证](https://github.com/xiaoland/velune/actions/runs/46100529533)

两份本地验收清单使用同一实际动态库（SHA-256 前缀 `ec8b7fa9cd41`）、配套生成模块和资源。默认运行曾因合成草稿 ID 无效被真实绑定拒绝。换为合法 UUID 后，默认运行的配置／草稿重开通过 3 项，历史压力通过 7 项，四个外部 CLI 用例明确跳过。随后显式提供 CLI，单独运行这四个用例，全部通过。

| 隔离验收 | 实际观察 |
| --- | --- |
| Claude 原生与托管上游 | 各 4 项。后台 Bash 跨前台轮次继续运行，后续输入复用同一子进程，任务回调形成独立回复。 |
| Codex 会话生命周期 | 8 项。冷重开、外部写锁释放、继续发送、队列只派发一次，以及停止后重开并再次发送。 |
| Codex → Pi 交接 | 10 项。加密上下文触发摘要，取消和失败保留队列，准备后只发队首，实际 Pi 收到摘要和输入各一次，冷重开能读到维护事件。 |

使用 Codex `0.171.0-alpha.13`、Claude Code `2.1.327`、Pi `1.4.0` 与 Node `24.22.1`。清单位于 `target/verification/acceptance/20261128T025524Z-fe63bd/manifest.json` 和 `20261128T025534Z-02ab3c/manifest.json`。另显式运行了四个已安装 Codex 的 Rust 读取／导入检查，全部通过。包含 Claude 空导入修复的 `c64ce84ce` 在本机干净工作树通过 820 项 Rust 检查，11 项按条件忽略。重新生成绑定后，六个验收场景在同一份清单中通过 36 项检查，清单为 `target/verification/acceptance/20261128T031250Z-57b3a1/manifest.json`。随后 Linux 在假进程追加 2 MiB JSON 时读到半条记录，报 `EOF while parsing a string`。夹具现在对同一文件使用读写双方遵守的文件锁，完整记录仍必须正常解析，未知请求仍必须恰好收到一次拒绝。在子进程持锁并分段写入的定向对照中，撤回读侧锁后，同一检查在第 1,024 字节报 JSON 截断。恢复锁后，同一检查读到完整记录并通过。该夹具只用于测试。最新提交的全工作区门禁汇总在 [PR #17](https://github.com/xiaoland/velune/pull/17)。

## 限制与后续边界

真实 CLI 使用隔离 HOME、合成会话和本机回环上游。本轮没有使用日常会话、真实账户或计费服务，真实服务端到端行为仍待相应场景验收。需要指定合成会话目录的 Pi SDK 历史夹具也未运行。Claude 空导入与并发升级实测为 2.1.327，最低支持版本、Windows 锁分支及真实断电恢复未实测。

在 Android／iOS 源码中未找到独立的 test source set／XCTest 套件。Android 有具名 debug gallery，Windows 主要依赖构建、token 对照和人工验收。本轮未执行这三个平台的原生验收。Mac 后台检查覆盖程序化检查入口，系统键盘、输入法、AX 与通知点击仍需单独操作验证。

Pi child-host 的旧插件末段只断言累计请求数，若干可选 SDK 反例只检查非零退出。这些测试仍提供部分保护，末轮请求与具体拒绝原因尚缺独立观测。links_metadata 组件测试覆盖已分阶段实现的存储与 lease 契约，application 的写入接线仍待验证。
