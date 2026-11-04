# 输入区、会话浏览与窗口连续性

2026-11-03 用户授权继续改进：接入可用的原生已读状态；composer 响应式利用横向空间，发往／附件宽时列排；明确无 project/cwd；去掉拥挤配置行上的草稿按钮；调查 2～3 秒会话打开；全屏源窗口新开仍为普通窗口，跨窗口拖入标签不制造额外全屏窗口。

沿用此前授权和证据，保留其他任务工作；不提交、不重装日常应用、不读取真实凭据或会话。验证不占用用户鼠标、键盘和前台，不用 computer-use；后台原生检查、离屏布局与合成 application 按各自边界组合，受影响路径足够即停。未改变持久格式，不需要迁移。

## 后续收敛

原生已读、0.166 兼容与后续 composer 居中／重启恢复已在[会话可靠性任务](../reliability-and-codex-166/packet.md)实现并验关键路径。下文的“尚未实施”是当时的调查状态；当前判断及交付以该任务为准。本包的导航、cwd 和全屏验证限制仍有效。

## 当前实现

- Composer 按自身可用宽度将发送目标置左、附件置右，编辑区上限 760pt；空间不足回到上下排列。去掉操作行的草稿按钮，已有保留输入入口移到会话菜单和输入区空白处右键菜单；普通未发送文本不需要另存。锁定只限制提交，编辑／附件继续保留。
- 工作目录菜单、命令面板明确提供“无工作区”。支持 optional cwd 的运行时仍传空；ACP／DSH 必须绝对目录时在执行机器上创建对应用户 HOME 的 Documents/Velune，并按无项目组织。
- 新标签复用 DeviceStore 已读列表，不先重新全量 list。已读、置顶、移组使用既有索引重新投影，保留上次历史失败；归档仍读取原生变动。单击双击判定原有语义不变。
- 拖放目标在开始异步加载 provider 之前显式取消 ListDragWatch 后备新窗口动作；松开时延后一个短周期，让同步接受动作优先。显式新开窗口首次展示禁用自动全屏，成为 key 后恢复其全屏能力；仅本次展示意图，不改恢复数据。全屏源窗口的新窗口使用正常默认高度。

## 已核实／待决

- 当前安装 Codex 0.159.3，协议无 readState；最新官方 0.166.0 有实验 readState：列表和 thread/read 返回 revision receipt，写入须 exact revision CAS，冲突不得盲重试。原文暂存 `/tmp/velune-navigation/{readme-166.md,thread-166.rs}`。现有其他运行时适配协议没有接入原生已读。
- 先前曾提问是否新增 0.166 类型适配器；用户追问兼容性后，撤回“必须新增类型”的未验证前提。现有 Codex variant 实际已覆盖 0.159.3～0.163，版本边界是验证范围；是否拆分须依据 Velune 消费契约的兼容结果决定。相关源升级仍未实施。

## 分层证据

1. 当前合成 Codex 核心读取会话 44～108ms（`/tmp/velune-navigation/baseline.log`），并未复现整体 UI 2～3 秒，不能外推整条 UI。
2. 给另一运行时 thread/list 注入 2 秒延迟：旧库 mark-unread 2.156s、mark-read 2.195s；新库分别 0.007s／0.004s，返回 unread 布尔正确；会话读取 0.153s。`perf-before.log`、`perf-after.log` 与 `organization-perf.py` 均在上述临时目录。只使用合成 HOME/agentDir，无真实调用。
3. Application release clippy（含 all-targets、warnings-as-errors）、cargo fmt --all --check 通过。最终 SVC Mac 构建通过，执行 `81688d25-901b-4c1a-88ec-8c2adb161836`；产物在 target/macos/Velune.app，未重装日常应用。检查辅助源及两个 --check-navigation-* hook 已删除；临时源留 `/tmp/velune-navigation/NavigationVisualCheck.swift`。
4. 实际 Mac ConversationStore 在已有列表时打开目标 0.137s；对另一个运行时的 2 秒慢扫描计数没有增加。只运行受影响路径。`native-loading.log`。
5. 离屏原生 NSHostingView 检查 composer 的 520/900/1280pt 宽度、共享窄布局 102pt 最小高度及宽布局 81pt 最小高度。遍历每个实际 host 的 NSTextView 记录边界，均在窗口内；锁定资格下调用原生文本编辑，模型同步成功且提交仍被禁止。所有窗口 isVisible=false，无鼠标、键盘注入或前台激活。截图在 `visual/{narrow,medium,wide,shared-minimum,minimum}.png`，尺寸记录 `visual/geometry.json`。已逐张查看主要宽窄与最小高度截图；横排附件不保留空辅助行，窄布局目标行计入最小高度，旧独立目标行的 22pt 补偿已删除。
6. cwd-check 用合成 ACP fixture 和实际 Codex 0.159.3 app-server 创建、不发送：ACP 收到 configured HOME/Documents/Velune 且目录已创建；Codex thread/start 未指定 cwd（原生仍可返回自己解析的实际目录）；显式目录原样保留。`cwd.log`、`cwd-check.py`。修复了首次检查发现的 ACP 无 agentDir 问题，目录创建直接使用 prepare_runtime_location。
7. 窗口补丁经最终构建确认，但没有操纵真实全屏 Space 或真实鼠标拖拽复演。单独 NSHostingView 脱离 SwiftUI Scene 时，windowFullScreenBehavior 不改变 NSWindow flags；该实验不构成全屏场景证据。不得宣称系统场景已实机验收。
8. 已更新 composer、conversations、workspace-layout、Mac/application README 与 S67。157 个相对链接／锚点通过，git diff --check 通过。无持久格式变动，不需要迁移。

## 剩余事项

原生已读接入尚未实施；当前运行时继续使用 Velune 的已读组织状态。0.166 支持须先核验并调整实际不兼容路径，不再以用户选择“新建 variant”作为已证实的必要前提。已核对本机 Pi 1.4 SessionManager 声明和 ACP 1.11 v1 schema，未提供标准 readState；不把任意 metadata 猜作已读权威。Codex 原文：[0.166 app-server README](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/app-server/README.md)。其余请求已实现并构建，窗口系统真实交互仍是明确的验证限制；不为此抢占用户桌面。

## 0.159.3 → 0.166.0 兼容性追问

用户询问是否确有 breaking changes，以及现有 adapter 为何不能支持 0.166。此轮仅调查与说明，没有改产品源代码或扩大版本区间。

- 直接拒绝来自 version.rs 的 `>=0.159.3,<0.164.0` 检查；`codex-0.159` 是现有类型身份，已通过同一 adapter 支持较新 0.163。不能仅凭新 minor 判断必须分 variant。
- 比较官方两个 tag：0.166 的 readState/readStates 是 optional 响应字段和新增实验 RPC／通知，没有删除原有列表／读取入口；该新增本身不迫使分 adapter。
- 确有相关行为差异：0.159 的 thread_resume 调用 load_for_cwd，0.166 改为 load_for_resume，先验证 saved/requested provider 均等于不含本次 resume 请求覆盖的 current provider；输入还通过 check_thread_model_provider_for_input 做同样比较。官方 README 的 Model provider admission 明确说明这项约束。
- Velune 现实现启动进程只加 app-server，在 thread/start/resume 的请求 config 中注入 model_provider=velune。若服务器配置仍选原有 provider，按 0.166 源码这条 managed 路径会触发 modelProviderMismatch；不能只提高 upper bound。需调查并调整 provider 注入／恢复和既有线程的接续。此结论来自双方源码对照，未运行实际 0.166 CLI 联调。
- 另一个上游不兼容例子：review/start 不再接受 detached，要求显式新建线程；Velune 不调用该接口，不是当前阻碍。
- 结论：先在现有 Codex adapter 上核验并收敛兼容差异，再根据实际共存能力决定是否需要独立 variant；之前直接要求“新增 0.166 adapter”证据不足。临时上游源码对照留 `/tmp/velune-navigation/compatibility`。
