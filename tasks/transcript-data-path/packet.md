# 会话数据通路性能

## 当前任务

2026-10-10 用户复验消息列表重写（见 [Mac 会话展示任务](../mac-conversation-usability/packet.md)）后认可性能明显改善，要求继续推进：懒加载（工作过程、工作过程中折叠的工具消息）、分页，以及 Harness 本地会话文件 ↔ Velune 这一层的性能。用户随后在“界面分页”与“数据分页”之间选择数据分页：滚动条仍代表完整会话，大纲跳转不需先加载；大负载按需读取，轮询只传变化。

授权沿用消息列表任务：可改 agent runtime／application／bindings／Mac，自主本地提交并重建安装；不 push、不新增自动化测试、不读取真实会话或凭据、不调用真实提供商。开发在独立 worktree 分支 `perf/transcript-data-path` 进行，避免混入其它会话的未提交工作。

## 基线测量（合成数据）

合成 Pi 1.0.2 会话目录：121 个会话共 252MB，最大会话 20MB／3200 条 Pi 记录（Velune 投影 2000 条消息）。release bindings 经 Python UniFFI 调用，暖文件缓存：

| 操作 | 改动前 |
|---|---|
| `list`（全部历史） | 944ms |
| 打开小会话 | 1142ms |
| 打开 20MB 会话 | 1256ms |
| 未变化轮询（20MB 会话） | 34ms；小会话约 3ms |

归因：

- 每次打开都先重新列出全部历史，仅为验证会话属于该实例；Pi SDK 的 `listAll` 逐行解析所有会话文件，成本随历史总量线性增长。
- 每次 Pi helper 启动都导入整个 SDK 入口（约 0.28s）。
- 轮询每次克隆完整快照（含全部工具输出），经 core JSON、api 反序列化、bindings 转换、UniFFI 四次全量转换；Mac 主线程还做全量相等比较。

## 文件系统层（已实施）

- application 按实例缓存最近一次列表（实例配置相同才有效）。打开来自列表的会话直接复用其来源范围成员关系；不在缓存中的标识重新列出，不接受任意路径。
- Pi 列表 helper 契约 v2：发现方式与 `SessionManager.listAll` 相同，按文件大小与纳秒修改时间（十进制字符串，避免 JSON 浮点精度误差）识别变化；只有新增或变化的文件经 Pi 自己的 `listAll` 读取（通过私有临时链接目录），Pi 仍是全部元数据字段的权威来源。未变化列表不导入 SDK。
- Pi helper 只导入版本固定的 `dist/core/session-manager.js`（已有 `importModule` 先例），不加载整个 SDK 入口。

结果：未变化列表 944→约 50ms；打开小会话 1142→约 125ms。逐字段核对：新 helper 对全部 121 个会话的输出与原 `listAll` 一致；追加 `session_info` 后下一次列表只重读该文件并得到新名称。

## 交付层（已实施）

- core 为每个展示视图（上下文运行时＋会话，最近 8 个）记录已交付内容：按消息相等性分配 stamp，游标为 generation（起点取时钟，旧 core 的游标不会误配）＋sequence。
- 所有快照结果（操作返回与完整 `snapshot`）中超过 2KB 的推理与工具输出只携带字节数；`message_payloads` 读取最近交付的完整内容。完整消息列表形状保留，既有人工验收脚本仅两处工具输出断言改为载荷枚举。
- `snapshot_update(since)`：未变化不序列化快照；变化时只传变化消息，`message_ids` 给出完整顺序。
- Mac：轮询携带游标，后台映射，主线程按身份合并；合并基准不符时忽略，下一次从当前游标继续，缺消息时回退完整快照。展开延迟内容时读取，读取结果按 stamp 保留在 Store，工作区广播合并时也保留；较新 stamp 时重新读取。
- 不做界面分页。工作过程与折叠工具在界面上本就只在展开时排版；数据层由载荷延迟承担。

## 结果（合成数据，同上）

| 操作 | 改动前 | 改动后 |
|---|---|---|
| 未变化列表 | 944ms | 约 50ms |
| 打开小会话 | 1142ms | 约 125ms |
| 打开 20MB 会话（core） | 1256ms | 246ms |
| 打开 20MB 会话（Mac Store 端到端） | — | 236ms，2000 行，1200 条消息含延迟载荷 |
| 未变化轮询（20MB） | 34ms，全量传输 | 约 5ms，无消息传输 |
| 读取 3 条延迟载荷 | — | 0.2ms |

## 验证

- 静态：`cargo fmt --all --check`、全 workspace／targets／features clippy `-D warnings`、bindings 无默认 features clippy、Swift debug 构建。
- `manual-transcript-scroll.py`：800 行快速上滑位移 0、最差单步约 22ms；新增延迟工具输出检查（展开即请求、载入后原位撑开且行顶不动、内联后不再请求）。
- `manual-transcript-outline.py`（实际 Store＋隔离 Pi 1.0.2＋合成上游）通过；临时插桩确认流式期间轮询走增量路径（如“变化 2／共 7”），插桩未提交。
- 临时 Store 检查（未提交）：真实 Store＋Transport＋core 打开 20MB 合成会话，按需载入 12KB 工具输出后轮询不丢失。
- `manual-session-browser.py`、`manual-pi-session-resync.py` 通过（使用本分支资源与绑定）。`manual-error-diagnostics.py` 的列表／读取失败原因部分通过；其后“系统通知不归入工作过程”断言在基线 `cc97cd5` 上同样失败，与本任务无关，未处理。`manual-pi-native-loop.py` 需要构建 manifest 的完整安装包，未运行。
- 未验证：真实 Harness 历史规模与物理滚动体验由用户验收；Codex／Claude／DSH 列表仍按其原有方式全量读取，只有打开时不再重复列出。

## 缺陷：展开工具消息覆盖下方兄弟行

用户反馈（2026-10-10）：展开工具消息后内容盖住下方未展开的消息。原因：虚拟列表物化行时，单元格总是采用该行精确排版，而行高只在若干显式步骤中提交；视口内或 AppKit `prepareContent(in:)` 预备区里尚未提交新高度的行（宽度或内容变化后）会以更高的排版画在旧行高里。修复：物化前在视口±半屏内提交所有不一致行高（保持阅读锚点，直至收敛）；预备区只物化行高已一致的行，不在预备内容时改变几何。`manual-transcript-scroll.py` 新增 TOOL-SIBLINGS 场景（工作过程内连续工具行逐个展开、延迟载荷载入、更新与稳定后检查），几何检查改为任何单元格子视图超出单元格即失败；窄宽度下旧实现可被该检查捕获（子视图 502 > 单元格 496），修复后全部通过。用户截图中的具体触发路径未在探针中原样复现，由用户验收。

## 构建与安装

2026-10-10 从源码 `af265fc` 的独立干净 worktree 完成 locked Rust release 与 Xcode SwiftPM release 构建、严格签名，安装至 `/Applications/Velune.app`（版本 `0.1 beta.1`）。manifest source 一致且 dirty=false；主二进制与 Rust 库 SHA-256 与签名构建包一致。安装前正常退出运行中的 Velune，安装后重新打开并确认进程运行、无崩溃报告。不 push。

2026-10-10 展开覆盖修复源码 `71e0575` 同样从干净 worktree 构建签名安装；manifest source 一致、dirty=false，主二进制 SHA-256 与构建包一致，正常退出后重装并重新打开，无崩溃报告。

## 后续候选

- 其他 Harness 列表的增量化需要各自来源契约，按实际规模再定。
- 已载入的超大工具输出仍整段排版；如真实使用中出现卡顿，再考虑截断显示（属于产品行为，需用户决定）。
