# Velune

这是用户自有 Velune coding agent 产品的独立文档仓库。Velune 的名称已经确定；当前只有产品知识与任务状态，没有产品实现。Beluna 是独立的、非 Coding 的通用 Agent / Digital Life 姐妹项目，不属于本仓库范围。

核心方向是多订阅、多 LLM 供应商、自动 LLM 路由、多 coding agent harness，以及跨 harness 会话的感知、交流和协作。远程、多设备和多平台能力是后续在此基础上的分布式扩展。

## 从这里继续

1. [AGENTS.md](AGENTS.md)：协作边界与文档更新规则
2. [产品定义](docs/prd/index.md)：已确认的目标、优先级、范围和术语
3. [架构方案](docs/design/architecture.md)：已认可的核心设计及 Rust／原生客户端细化建议
4. [当前 Task Packet](tasks/harness-routing-feasibility/packet.md)：研究状态、验证队列和未决项
5. [来源索引](docs/sources.md)：关键用户决定的消息 ID；不复制完整聊天

完整导航见 [docs/index.md](docs/index.md)。

## 当前状态

- 已初始化独立本地 Git 仓库，分支为 `main`
- 按 xiaoland/svc 的知识归属和 Task Packet 方法组织文档，不依赖它的 CLI 或运行时
- 已建立共享 Agent 指引、产品知识与下一步 Task Packet，由 Git 记录版本
- 用户已认可核心产品设计、MCP／ACP 分工，以及 Rust＋SQLite、Apple／Android 原生 UI 方向
- 已按复核澄清实际协作机制与持久化支撑的区别；Apple 桌面、FFI、SDK 桥等具体边界仍待原型验证
- 架构是设计产出，没有实现、真实账户登录或模型运行验证；ACPHub 精确源码尚不可访问
- 本仓库用于承载已发布的文档快照；不包含原工作区的 `.git` 对象或历史

用户要求以持久化仓库承载后续讨论，不交付压缩包。远端地址为 <https://github.com/xiaoland/velune>；本次导入是重建文档快照，不应描述为原提交或原工作区的逐字节复原。
