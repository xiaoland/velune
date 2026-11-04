# 图片打开与 Codex 会话命名

用户授权修复三项：粘贴图片可通过既有系统文件打开机制查看；Velune 创建的 Codex 会话自动命名；子 Agent 会话不再遗漏原生名称。延续上一轮的分层、关键路径验收要求，且交付须说明每项如何修复。

当前：三项源码已实现，实际 Codex 合成联调及 Mac 原生关键路径已通过；未提交、未重装日常应用。图片原已写为普通本地 PNG，缺的是 composer 附件 chip 打开入口；现在所有附件共用 NSWorkspace.open，移除资格单独判断。Codex 原生 name／agentNickname／preview／agentRole 在列表、历史与 resume 使用同一解析；未命名列表项不再覆盖工具链接名称。

实现依据：Codex 0.159.3 的自动命名由客户端发起临时结构化线程，再写回 thread/name/set（[上游实现](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/tui/src/app/thread_title.rs)）；Velune 原只控制 app-server，因此缺少触发端。已补异步、有界、一次性的命名，沿用首轮模型连接，临时线程不进入正文或子 Agent，手动名称优先并消费实时名称事件。仅取合成数据验证，不读取真实 HOME／凭据／会话、不调用公网模型，不提交或重装日常应用；保留工作区其他任务改动。

验收边界：局部名称优先级／异步结果边界，真实 Codex＋合成回环模型的一条自动命名路径，Mac 原生粘贴图片打开及名称显示。只补受本次变更影响的证据，复用旧 composer 验收，不重跑完整布局矩阵。

本轮日志／临时脚本归 `/tmp/velune-codex-titles`。源码构建与联调分开记录，编译不作为 UI 证据。先前 composer 的布局、锁定与恢复结果继续使用原任务记录，本任务不重新覆盖那些路径。

## 分层证据

- 局部规则：名称优先级覆盖手动名称、无 preview 的原生昵称、首条消息和原生角色；初次检查通过，最后角色来源类型修正后仅补跑这一项，已通过（1 项，28 项未运行），日志 `name-check-final.log`。Rust application／agent-runtime release clippy（仅相关 lib，`-D warnings`）通过；最新动态库 release 构建通过，fmt 和 diff 检查通过。
- 真实组合路径：Codex CLI 0.159.3＋实际 application＋独立 HOME／VELUNE_HOME＋回环 Responses 模型。自动标题写回原生；真实 spawn_agent 产生昵称 Heisenberg，列表、关联 Agent 与打开后的投影一致；延迟自动命名不覆盖手动标题；新 application 进程读回两类标题；只留下父、子与手动测试会话，没有持久命名线程；两次命名请求实际暴露的工具列表均为空。`/tmp/velune-codex-titles/e2e-5.log`，数据根 `/tmp/velune-native-titles-v3wjnf_d`。
- 原生 Mac：隔离副本附件有独立打开／移除入口，点击合成 PNG 后系统“预览”显示同一文件 URL 和图像，附件仍在草稿中。实际合成配置启动后，列表／标签显示“原生自动标题”；关联 Agent 显示 Heisenberg，打开后标签和窗口仍为 Heisenberg，子会话保持只读。均通过 CUA 在原生应用操作与截图观察，没有使用 Windows App。
- 构建：SVC Mac `e3ddf057-4956-4e7a-926f-50f0f3e47bb8` 通过。首个构建在重新编译绑定生成器阶段主动中止，随后通过临时 PATH wrapper 复用源码未变、固定 UniFFI 0.32.2 的已构建生成器，仍从新库生成 Swift 绑定并编译／签名应用；没有修改仓库构建配置。最后仅修正“附件清单不算用户文字”的 guard，单独重建动态库并更新／签名隔离副本，复用未变 Swift 构建。日志为 `build-macos-final.log`、`native-final-build.log`、`clippy-final.log`。

合成联调曾发现命名线程禁用宿主 MCP 时丢失传输字段，已保留完整配置再单独禁用；失败保持原会话与首句标题。另修正临时脚本的旧工具命名（当前为 multi_agent_v1 namespace）及 ListedConversation 的类型访问，最后组合路径通过。早期失败日志仅作诊断，不计为通过证据。

## 边界与收尾

本轮未改图片落盘与粘贴识别，复用上一轮原生命名 pasteboard 的粘贴落盘证据；本轮原生只补通用附件 URL 的点击打开，没有重复整套粘贴／布局矩阵。命名链路覆盖当前支持的 Codex0.159.3，未宣称其他运行时新增自动命名或其他客户端图片 UI 通过。本轮也未验证公网模型或真实账户。

原生合成图片窗口已关闭，验证应用副本已正常退出，临时脚本和日志留在 `/tmp/velune-codex-titles`；日常应用与其他任务改动保留。权威行为同步至 PRD／S64，运行时实现说明归 agent-runtime README。文档相对链接／锚点检查覆盖6份文件132处，无错误。
