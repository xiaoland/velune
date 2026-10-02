# Velune

这是用户自有 Velune coding agent 产品的独立仓库。Velune 的名称已经确定；当前有无凭据 Rust＋SQLite 核心原型及产品知识。Beluna 是独立的、非 Coding 的通用 Agent / Digital Life 姐妹项目，不属于本仓库范围。

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
- 已运行本地确定性模拟闭环；没有真实 Harness、账户登录或模型运行验证；ACPHub 精确源码尚不可访问
- 本仓库用于承载已发布的文档快照；不包含原工作区的 `.git` 对象或历史

用户要求以持久化仓库承载后续讨论，不交付压缩包。远端地址为 <https://github.com/xiaoland/velune>；本次导入是重建文档快照，不应描述为原提交或原工作区的逐字节复原。

## 运行无凭据原型

使用官方 Rust 1.99.0 与本机 C 编译工具（SQLite bundled 需要）：

```sh
cargo test --locked
cargo run --locked -- demo /tmp/velune-demo.sqlite
cargo run --locked -- inspect /tmp/velune-demo.sqlite
```

`demo` 要求数据库不存在，输出 `simulation: true` 的 JSON：Codex 模拟会话向 Claude Code／Pi 模拟会话委派合成检查，结果返回 Codex 并验收。`inspect` 取得独占宿主后执行重启恢复并输出诊断，不是只读数据库工具。所有模型请求都在进程内确定性模拟；未启动真实 Harness、网络监听或模型 API。

完整构建、故障证据、Mac mini 原生体验接线及真实联调准备见 [开发说明](docs/development.md)。

Mac mini 首个原生体验已有 [AppKit 源码](native/macos/main.swift) 与 [本机构建脚本](scripts/build-macos.sh)。独立 Rust Host 经私有 Unix socket 操作同一核心，界面退出不停止 Host；设备侧编译和用户验收仍待执行。
