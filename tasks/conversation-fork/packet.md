# 会话 fork：不等待源会话空闲

用户要求从已完成 turn 立即创建独立分支，即使源会话的后续 turn 仍在执行。用户已授权实现和提交；不授权重装日常应用，也不在日常会话上做 fork、迁移、解锁或中断。

## 当前行为与实现

普通 fork 包含所选 turn 的完成结果，不发送新消息；编辑后 fork 复制被替换消息之前的上下文，再发送替换消息。两者都不等待、停止或切换源会话。只有原地编辑仍按确认语义先停止源 turn；未完成的 turn 本身不获得已完成边界资格。

- Codex 以 `ConversationStart::CodexFork` 复用 native／managed 的模型、网关和 MCP 装配，在独立连接中直接 `thread/fork`，只接管返回的新目标，不 invalidate 或 resume 源 writer。
- Pi 用独立 SDK SessionManager 复制固定原生 entry 前缀，再准备目标；不配置源模型，不对源宿主调用会切换／abort 的 RPC fork／clone，源文件保持只读。
- Claude 在独立执行中保存固定 from／at。命名区分目标快照和 RPC 子进程实际绑定的身份，未首发名称保存在 Fresh／Fork 启动状态，已落盘且未绑定的目标通过原生元数据入口重命名。
- Mac 删除 heldFork、heldEditFork、forkReady、等待提示和 turn 结束后重试；保留确认、实际复制加载态、防重复请求和同设备新标签页。
- 只读／外部控制源仍保留已完成轮次的 fork。发送、继续和原地编辑仍受源控制权限制；显式控制交接策略不变。

产品规则见 `docs/prd/conversation-view.md`；实现入口和约束见 application、agent-runtime 与 Mac README。上一轮 `1ecfb16` 已提交确认、进度、命名、原生优先兜底和新分支配置即时就绪。

## 验收

工作树检查通过：两个生产库严格 Clippy，application 97 项、agent-runtime 60 项完整库测试；Mac 最终 SVC 构建输入前后一致，running／elsewhere 源立即派发回归、同设备快照接收、配置即时调整和离屏加载态检查通过。

真实 CLI、合成 HOME 和回环上游的 11 个并发栅栏场景通过：Codex 默认／显式 native／managed／编辑 fork，Pi native／managed／编辑／首条编辑 fork，Claude managed／编辑／首条编辑 fork。源请求保持挂起时 fork 已返回；源 PID 不变，释放栅栏后自然完成；目标不含源后续轮次且模型选择正确。Pi 四例源字节不变，Claude 三例核对原生名称落盘。版本为 Codex 0.167.0-alpha.3.2、Pi 1.4.0、Claude Code 2.1.292。

最终矩阵及重跑说明在 `target/verification/conversation-fork/concurrent/`；历史失败单独保存，不代表最终结果。只读资格检查覆盖五种访问受限状态、完成／未完成 turn、交付快照和源不变，证据在 `target/verification/conversation-fork/readonly-source/`。

## 提交与边界

已从 `d6753a2` 提取本轮独立补丁，避免混入并行编辑器、远控、共享输入和其它运行时任务。隔离补丁的两个生产库严格 Clippy、agent-runtime 40 项测试和 11 个真实 CLI 并发场景通过；application 65 项通过，1 项输入迁移断言失败。移除补丁后已在同一基线复现该失败，不是本轮回归。独立 Mac 构建被未修改的基线代码阻断：`Problems.swift` 初始化缺少 peerId；仅在隔离副本补齐后，又暴露 `DeviceStore.swift` 与 Transport／导航之间的既有接口缺口。未将这些修补纳入 fork 提交，独立产物未运行 UI 检查；Mac 使用上文集成工作树已通过的构建和隐藏检查证据，不宣称独立基线构建通过。

独立补丁证据：`concurrent/commit-isolation.log`、`commit-baseline.log`、`commit-cli/summary.json` 和 `commit-macos.log`。并发脚本在基线 API 上只省略未请求的未来编辑器字段，实际边界、源继续、模型和名称的判断标准没有改变。

本轮按授权交付独立提交，只包含上述 fork 修改，不推送、不重装日常应用。真实会话没有被操作；临时夹具、子进程和隔离 worktree 随提交收尾清理。已更新 fork 全景 artifact：`.ryugi/artifacts/document/conversation-fork/doc.xml`。它同时标明并行工作树中的其它编辑扩展，不把引用式回退混称为本轮已验收的原生消息翻译。

Claude 尚未首发的分支仍依赖进程内准备状态，不保证跨重启恢复；名称后缀只避开已知列表和已打开会话，不承诺全局唯一。其它平台未取得与 Mac 相同的按钮交互验收。全目标 lint 不在本轮“两个生产库严格 Clippy”结论内。
