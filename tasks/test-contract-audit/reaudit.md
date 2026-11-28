# 测试与验收重审

这次重审直接改变了三条使用路径。ACP 创建或打开被对端拒绝时会撤销尚未确认的身份和历史；Mac 在聚合窗口之间拖放列表行时按来源设备定位会话；选择迁移脚本作为唯一改动的 PR 也会运行消费这些脚本的 Rust 检查。前一轮已交付的 Pi 背压、IPv6 端点、Claude 空会话恢复和验收结果归属修复保留在同一 PR，证据见[首轮报告](audit.md)。

## 范围与基点

比较基线为远端 develop `d2c8e35c0`。重审开始于 `b47863cf2`，包含首轮 PR 与上游的合并。按生产调用方分配了 354 个运行时、339 个应用与绑定、143 个服务与存储 Rust 声明，逐项读断言及参数表。还检查了生成绑定的场景、独立 Python/Node 检查、原生平台预览与调度、手工验收、CI、打包及供应源码补丁的本仓库消费者。原生操作、构建机以及真实服务各有自己的执行条件。

[逐声明表](declarations.tsv) 保留了 836 个声明的基点、初判和处理依据。逐声明表经接收方复核后有 689 项初判保留、35 项补强、94 项合并候选和 18 项退役候选。第二次沿调用点、README、历史与接收方查证，再决定迁移、退役或保留。平台及脚本另列到[平台与参数行台账](platform-checks.tsv)，保留初判与参数差异，实施与运行结论以本文为准。运行时发现两份 Codex 测试均直接启动同一 Python helper，撤回了此前“不同测试层”的判断；又发现读取早期 Codex 记录仍需要 Rust 的双端迭代器，保留该方法。宏生成的跨语言导出、已公开说明的接口以及真正用来控制并发的夹具也按各自承诺保留。[运行时保留位置](runtime-keepers.md)

采用 Caldo 的删除标准、abyss 公开提交中的 test-hygiene 与 test-adequacy，以及 OpenClaw 公开整合的逐声明审计、分层方案、迁移、独立保全复核、故障对照和最终对账流程。资料边界为这些公开版本。Velune 使用本仓库的 Cargo、SVC、生成绑定及原生门禁。[prune-tests](https://github.com/caldoai/skills/blob/d5b68dace378219890d821adb3685538e2c69b04/skills/prune-tests/SKILL.md) · [abyss 425a286](https://github.com/BeautyyuYanli/abyss/commit/425a2863549692f21728502eb698086e8c3ad64d) · [test-audit](https://github.com/openclaw/openclaw/blob/beb28f6f7ad81121da37a11907002265fb039474/.agents/skills/test-audit/SKILL.md) · [CAMPAIGN](https://github.com/openclaw/openclaw/blob/beb28f6f7ad81121da37a11907002265fb039474/.agents/skills/test-audit/CAMPAIGN.md) · [候选证据](https://github.com/openclaw/openclaw/blob/beb28f6f7ad81121da37a11907002265fb039474/.agents/skills/test-audit/references/audit.md) · [验证方针](https://github.com/openclaw/openclaw/blob/beb28f6f7ad81121da37a11907002265fb039474/.agents/skills/openclaw-testing/SKILL.md) · [边界场景](https://github.com/openclaw/openclaw/blob/beb28f6f7ad81121da37a11907002265fb039474/docs/help/testing/boundary-scenarios.md) · [夹具](https://github.com/openclaw/openclaw/blob/beb28f6f7ad81121da37a11907002265fb039474/docs/help/testing/fixtures.md) · [测试编写](https://github.com/openclaw/openclaw/blob/beb28f6f7ad81121da37a11907002265fb039474/docs/help/testing/writing-tests.md)

## 各层的改动与剩余保护

| 负责的行为 | 删除或纠正的内容 | 谁继续证明该契约 |
| --- | --- | --- |
| ACP、DeepSeek、Claude 会话 | 退役只由测试显式开启的异步命令端口、票据、代际与创建预算。应用实际使用同步会话、poll 及协议回复。 | 在正常入口与合成子进程交换真实帧，验证未确认会话、已确认配置、请求 ID、拒绝原因、控件与终态。共享物理写入、回复预留、读取信用和冷历史各由传输 owner 继续保护。 |
| Pi 与 Codex 命令 | 退役无生产调用的阻塞便捷方法、测试专用 Start 和两层 Codex 导入转发。保留实际 Pi 命令端口和 Codex Resume。 | 正常创建、切换与导入，队列和管道的真实回执，保留回复、限制子任务停止范围、取消后可重发。官方 Pi SDK 重开观察精确历史、外部变更与备份原字节。 |
| 关联写入、草稿与配置 | 删除从未装入运行路径的异步关联写入器、对应测试和专属 HomeLock 分支。删除两个只为测试存在的应用包装。 | 真正 Repository 写入关闭再重开，核对复杂 ID、无关记录、旧 schema 迁移、冲突拒写和备份。保留继承 FD 解锁与待清理执行的独立锁契约。草稿目标、选择范围和后台手工读标志在调用方边界有固定样例；旧前缀指纹来自独立旧格式字节。 |
| 账户、路由与信任 | 将 profile、reset 的检查移到实际 collector；去除未使用的网关路由替换和信任服务转发。原授信字段、版本和持久化义务迁入真实读写者。 | 固定 profile 数值和错误来源、真实 loopback HTTP 的跳转/重试与 SSE 终态、两轮 Ark 工具调用及返回、Claude bridge 缓存的账户切换与中断恢复、旧版远端读写兼容各有独立检查。 |
| 生成绑定与结果解释 | 将缓存的 8 视图个数限制与 64 MiB 字节限制拆成可区别的输入。删除读取内部日志细节、复制服务内部模型编码和没有读取者的进度文件。 | 生成的 Python 绑定和真实核心库以请求记录观察冷读；实际 Codex 和 Pi、回环 provider 检查摘要模型、失败原因、队列和恢复。runner 用合法结果逐字段破坏，并分别检查非零进程退出和业务失败。 |
| Mac 原生界面 | 收敛只读私有状态的探针、手造接受计数、重复离窗布局和预览入口；补控制多设备拖放。 | 真实 AppKit 窗口、单元格、AX、coder 和 Core 使用者继续检查布局、对象顺序、恢复元数据、窗口或编辑器关闭后的迟到结果。已挂窗、离窗、独立进程重开分别保留。 |
| 平台、迁移与发布 | iOS 消息删除 delta 要同时验证未变回复；修正 Mac wrapper 错配参数、Pi 负向错误阶段和 CI shell 检查每个文件。迁移备份直接对照两个原输入文件。 | iOS Debug 预览、Mac 参数拒绝及原生子进程、实际安装的 Pi 与旧插件、迁移脚本真实写盘及重新运行。Android Gallery、Windows parity、包签名与发布资产检查按场景保留。 |

## 独立复核与对照

- 缓存故意移除字节限制、将数量限制从 8 改为 7，旧验收都通过。新验收分别在目标观察失败。让摘要失败返回另一错误码也得到旧绿、新红。恢复源码并重新构建后两个绑定场景均通过。
- 让验收父进程接受其它用例身份、或将 exit 0 的业务失败改写为成功，各得到旧绿、新红、还原绿。保留生成模块与库版本核验、真实子进程和产物清单。
- 将运行时策略的备份字节替换为 `{}`，或只把预期的策略冲突诊断换为无关前置错误，旧迁移检查都通过、新检查都失败。恢复后三个迁移检查通过。实际文件的原始期望取自产生迁移前。
- Pi 的过期摘要故意返回无关错误，旧 SDK 删除脚本通过，新脚本在原因断言失败；正常 SDK 上的删除、编辑、通知取消三个脚本均通过。Pi 插件检查还单独破坏了旧插件本轮回复、SDK 加载与 stop 回应；原检查把错误算作成功，新检查命中对应阶段。
- 独立检查删除的 Rust 覆盖时，找回 Claude 9 MiB 消息的完整协议帧、侧会话写入 context.txt 并连续提问、模型/effort/自动压缩启动参数与清除行为。这些已迁到实际 NativeSession。Mac 复核找回共享 ToolHeader 的动作和几何边界、原生项目目录拒绝条件及退役参数的显式退出。
