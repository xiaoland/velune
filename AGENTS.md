# Velune：共享协作指引

本仓库承载用户自有 Velune coding agent 产品。Velune 是已确定的项目名；Beluna 是独立的、非 Coding 的通用 Agent / Digital Life 姐妹项目。

## 先读什么

1. `README.md`：入口与当前阶段
2. `docs/prd/index.md`：已确认的产品意图
3. `tasks/harness-routing-feasibility/packet.md`：当前任务、未知项和下一步
4. `docs/design/architecture.md`：已认可方向与具体技术推荐，区分已确认和待验证；不等于已授权实现
5. 需要核对决定出处时读 `docs/sources.md`

## 知识归属

- `docs/prd/index.md` 是产品目的、优先级、范围和稳定术语的唯一权威入口
- `docs/sources.md` 只保存来源定位和精简释义，不复制整段聊天或重复维护需求
- `tasks/` 保存易变的 Task Packet；问题、证据、假设、工作计划先留在任务里
- `docs/design/architecture.md` 归属跨单元方案与拟议契约，显式标记复核状态；证据快照与试验缺口仍留在 Task Packet
- 尚无单元实现或产品运行环境，不建立空的 TDD、部署章节
- 将来出现独立知识归属与真实使用者，再依 SVC Corpus 引入相应规范；附近的 `AGENTS.md` 只补充本子树规则

## 必须保持的边界

- 核心优先级由 PRD 定义；不要恢复已被纠正的“手机优先／远程控制优先”路线
- “路由”仅指 LLM 请求路由；设备选择与 harness 调度不是同一个决定
- 首批 harness 固定为 Codex、Claude Code、Pi；以接管三者 LLM 路由为设计目标，研究实现路径，不把能力矩阵变成重新选择 harness 的闸门；排除 GitHub Copilot
- 已确认 Rust 主核心、SQLite、原生 Apple Swift／UIKit 与 Android Kotlin／Jetpack Compose 方向；不恢复 TypeScript 主核心或 Web 前端路线
- MCP 暴露协作工具、ACP 可替换接入已获认可；持久化是协作支撑，不代替发现、注入、委派、结果与失败协调机制
- Apple 具体平台覆盖、Mac Catalyst／AppKit、FFI 与运行宿主等尚有技术边界，不能把 native UI 当手机可以常驻运行桌面 Harness 的证明
- 不把聊天中的示例、助手提议或未核实的技术说法提升为已批准架构
- HAPI、Lody 只作说明性参考；Claude Code 订阅认证可研究其方案，但不意味着已允许复用凭据或认可可行性
- 不承诺订阅可以无限使用、免 token 计费、跨任意工具互通或规避供应商限制
- 不读取、复制或提交登录令牌、账户会话、密码、密钥、支付数据或真实会话记录
- 当前文档仍只描述调研与方案收敛；不实现产品、不推进 Factory26、不部署、不调用模型 API。远端发布仅限用户明确授权的 Velune 文档快照，不扩展为其他项目或服务
- 只采用 xiaoland/svc 的文档导航、知识归属和 Task Packet 方法；不要引入其 CLI、安装配置、运行时或其他额外工程
- 用户要求持久化仓库而非压缩包交付；本地 Git 提交不自动保证工作区或远端持久性

## 工作方式

- 开始非平凡任务前建立或恢复 Task Packet。先保持一个简短 `packet.md`，实际需要时再增长
- 严格区分：用户已确认的产品意图、已验证事实、待验证假设、候选方案、待用户决定
- 新事实给出来源、版本或提交、检查日期与适用边界；技术可行性和供应商规则以当时官方资料复核
- 用户作出新决定后，先更新唯一的长期知识归属，再更新 Task Packet，最后同步索引或短摘要
- 任务状态可以纳入 Git 以便跨环境恢复，但不是长期项目规范。任务关闭前确认持久结论已归位；获得正常删除授权后删除已关闭 packet，不另建任务档案库
- 共享规则放本文件；机器绝对路径、工具安装和临时操作记录不进入共享知识
- 文档编辑后检查相对链接和标题锚点，执行 `git diff --check`、`git diff --cached --check`，提交前核对 `git status`
- 当前没有产品构建／测试流程，也不需要安装任何文档 CLI 才能继续工作
