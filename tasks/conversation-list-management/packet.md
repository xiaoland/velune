# 会话列表管理

2026-10-14 用户要求改进会话列表（分组体验、分页加载、总数限制、项目名称显示）与会话归档，随后补充已读／未读、移动项目、拷贝深链／ID、置顶。用户的选择见 [S24](../../docs/sources.md#s24)，产品归 [PRD](../../docs/prd/index.md#会话内容展示) 会话浏览段落。

## 开发方对补充项的默认解释（待用户验收）

补充四项只有名称，按以下默认实现，用户验收时可改：

- 已读／未读：Velune 记录每个会话最后一次被看到时的更新时间；之后有新活动（含在别处运行的轮次）即为未读，行首显示圆点。打开并显示会话即为已读，可在菜单中手动标为已读／未读。功能首次启用前已有的会话视为已读，不让全部历史一下变成未读。
- 移动项目：只改变 Velune 列表的项目归属，不改变原生工作目录，也不改变下一轮的执行位置；可移回原工作目录对应的项目。
- 拷贝 ID：拷贝原生会话 ID（运行时 resume 接受的标识）；拷贝链接：`velune://conversation?runtime=<实例>&id=<会话>`，应用注册该 scheme，打开时在当前窗口打开会话。
- 置顶：Velune 记录；置顶会话显示在列表最上方独立的“置顶”组，不占总数上限，归档时取消置顶。

## 已核实的原生事实（2026-10-14）

- Codex 0.159.3 app-server：稳定接口有 `thread/archive`、`thread/unarchive`、`thread/list` 的 `archived`、`cwd` 过滤、`limit`／`cursor` 分页、`sortKey`、`useStateDbOnly`；`thread/list` 由状态库支撑，按页读取开销低。`project/*`（带 `projectId` 的项目注册）只在 experimental 接口中，是 Codex 自己的项目概念，不作为 Velune 依赖；`threadSection` 是稳定接口但属于 Codex 自己的分区。没有原生置顶或已读状态。
- Claude Code 2.1.286：本地会话按工作目录存放在 `projects/<编码后的 cwd>/<uuid>.jsonl`；目录编码有损，真实 cwd 取自会话记录。归档只存在于远程／云端会话，本地 JSONL 没有。
- Pi 1.0.2：默认按工作目录分目录存放会话，会话头记录 cwd；没有归档。自定义 `sessionDir` 时所有会话在同一目录，没有按项目的存放。
- 现状：`list()` 每次读出所有启用实例的全部会话；Claude 每次完整解析所有 JSONL，Pi 只重读变化的文件，Codex 逐页读完全部 thread。

## 方案

列表的分组、排序、筛选、总数上限与各组页数改由 core 计算，`list()` 只返回需要显示的会话（置顶、各组已加载部分、当前打开的会话），另返回组描述（键、标题／别名、总数、最近活动、是否折叠）与项目清单。搜索在 core 中按需读出全部匹配，单独调用，不改变全局窗口。

组织记录保存在 `VELUNE_HOME/conversation-organization.json`：置顶、Velune 归档、已读标记、项目归属、项目别名。折叠状态并入浏览偏好；总数上限是新的应用设置，原“每组首批”改为“每次加载”。

分两步实施：

1. 组织记录与窗口：core 窗口与组描述（先基于现有完整列表）、置顶、归档（Codex 原生，其余记录）、已读／未读、项目别名、移到项目、拷贝 ID／链接与 URL scheme、可折叠组、加载更多。
2. 按需读取：Claude 与 Pi 以目录与文件状态建索引，只解析需要显示的会话（按文件状态缓存）；Codex 保持状态库分页。显示的时间仍取自原生记录，文件时间只用来决定先读哪些。

## 验证

静态检查、构建、临时 HOME 的合成会话手工脚本（`scripts/` 下），覆盖窗口分配、折叠、加载更多、置顶、归档／取消归档（含 Codex 原生）、已读标记、别名与移动、按需读取的读取范围。真实会话与界面体验由用户验收。

## 进度

- 已记录决定（S24、PRD）与本 packet。
- 第一步完成：core 窗口与组描述（`local/organization.rs`）、组织记录文件、Codex 原生归档（`ThreadChange`、`list_archived`）、列表外会话查找、搜索（读最近一次列表）、bindings 与 Mac 侧栏（可折叠组、加载更多、未读圆点、置顶组、行菜单、项目重命名、已归档视图、设置中的两项数量、`velune` URL scheme）。协作工具改为读取全部会话，不受列表窗口限制。
- 验证：fmt／check／strict clippy／no-default-features、Mac 构建零警告；`manual-conversation-list.py` 全部通过（窗口分配、折叠、置顶、已读、移动、别名与搜索、Velune 与 Codex 归档、打开已归档 Codex 会话、重开持久化）；回归 acp、selection、codex-history、continuation、subagents、live、attach、collab、parallel、session-browser 通过。隔离 HOME 后台启动截图核对了置顶组、项目别名、未读圆点、组计数，并用 AX 操作“加载更多”。
- 第二步完成：Claude Code 与 Pi 先建索引（`browsing.rs` 的 `index_history`；Claude helper `index`／`summaries` 操作，Pi helper 契约 v3 返回全部文件状态与会话头、只解析 `want` 中的文件），列表以未读会话的文件时间与会话头参与分组、计数与窗口分配，`browse_read` 只读取窗口显示到的会话并重排至稳定；搜索读出全部未读会话，查找按 ID 读取单个会话，协作工具读取全部。Codex 仍由状态库列出全部 thread。
- 第二步验证：静态检查与 Mac 构建通过；`manual-conversation-list.py` 新增 `lazy`（Pi：初次窗口与“加载更多”只请求显示的文件）与 `claude_lazy`（合成 JSONL：文件最新但记录最旧的会话读取后移出窗口，补读下一条；加载更多、搜索、查找各只读所需会话），全部通过；上述回归与 `manual-session-browser.py` 通过。Claude helper 删除已无调用的 `list` 操作，`manual-runtime-provider-ssh.py` 改用 `index`／`summaries`，并补上它原先缺少的 `runtimeId`（此前即失败）。
- 2026-10-14 安装：两步合计以 `e17239d` 构建安装至 `/Applications/Velune.app`（0.1 beta.1），签名校验通过，manifest 提交一致且无未提交改动，后台启动无崩溃报告。
- 2026-10-14 用户反馈：Codex 不指定工作目录的模式是在 Codex 管理的位置创建会话，这些会话以及其他运行时的类似会话应归入无项目组。核实 ChatGPT 26.1007.30912 内置 Codex：无项目聊天（界面称“聊天”／Chats，`workspaceKind: projectless`）的根目录默认 `~/Documents/Codex`，可在其应用设置 `projectlessWorkspaceRoot` 中修改；每个聊天的 cwd 为 `<根>/<YYYY-MM-DD>/<标题 slug>`；其侧栏把 cwd 为 `~` 或在该根目录下的会话归入 Chats。app-server 0.159.3 的 `Thread` 没有无项目标记。Claude Code 2.1.286 与 Pi 1.0.2 没有类似的托管目录（Claude Code 只有“会话没有工作目录”的情形，Velune 已归入无项目）；Velune 自己在本机不指定目录新建的会话落在应用进程的工作目录（`/`），SSH 上落在远端 HOME。
- 实现：版本描述新增 `projectless_directories`（Codex 为 `Documents/Codex`），列表以运行时看到的 HOME（指定或托管 HOME；本机继承用应用的 HOME；SSH 继承 HOME 时探测一次并按执行设置缓存）判断，cwd 为文件系统根、HOME 或在这些目录下的会话没有工作目录项目，归入“无项目”组（Mac 将“未指定项目”改名为“无项目”）；移到项目后可移回无项目。不读取 ChatGPT 应用设置，自定义的无项目根目录不会被识别。新建会话的“最近目录”改取会话的项目，不再推荐 Codex 聊天的临时目录。顺带修复总数上限分配：此前每组都从总数扣去整页，组少于一页时也扣，后面的组可能一条都不加载；现在只扣实际加载的数量。
- 验证：fmt／check／strict clippy／no-default-features、Mac 构建零警告；`manual-conversation-list.py` 新增 `projectless`（Codex 聊天目录与 HOME 下的 Codex 线程、HOME 与根目录下的 Pi 会话归入无项目组，Pi 在 `Documents/Codex` 下的会话仍是项目；以较小总数核对上述分配修复；移到项目并移回）全部通过，`manual-session-browser.py` 通过。SSH 继承 HOME 的探测未实际连接验证。
- 未验证：向 Codex 原生归档的会话发送消息时 Codex 的行为；右键菜单、折叠三角、链接打开与 NSOpenPanel 需用户界面验收（自动检查会把应用带到前台）。
- 既有脚本漂移（与本任务无关，未修）：`manual-browser-preferences.py`／`manual-session-loading.py` 的 Swift harness 缺少后来加入的 Store 依赖文件；`manual-runtime-discovery.py` 第 52 行外部 Pi 路径断言不再成立。
