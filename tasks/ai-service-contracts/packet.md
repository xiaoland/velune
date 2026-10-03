# AI service：有界 MiniMax 流式实现与 fixture 验收

- 基线：`7c6f98267a56101d2117311adeaf10b50b01d8f2`（dev/ai-service-contracts），新分支 `dev/minimax-stream-fixtures`
- 设计权威：[AI service](../../docs/design/ai-service.md)，用户追加授权优先于此前仅类型范围
- 授权：最小直接派发 service、MiniMax adapter、合成文本／工具流真实 fixture、手动离线重复回放；普通 push 新 dev 分支
- 排除：新测试／框架、cargo test、路由／fallback 算法、Ark／Bailian、Host／UI／Mac／部署、PR／main／force
- 状态：实现及静态检查进行中；尚未采集本步 live fixture，不把预期当运行结果

## 网络与预算

同一个已验证 Cloud 环境，三个 Networksecret 变量存在；仅在手动 live 入口内存读取 MINIMAX_API_KEY，不输出／落盘值。目标固定 `https://api.minimax.cn/v1/chat/completions`、MiniMax-M3 普通 API，thinking disabled，standard 服务层，无内置工具／重试／redirect。

用户总上限 ¥5，历史 literal-name 401 的用量／扣费 unknown；本会话认证成功 165 input、2 output。保守账本预留历史 401 ¥0.80、认证 ¥0.10，每个新增 attempt ¥0.50，最多获准 8 次为 ¥4.90。当前手动入口进一步缩小为两个固定 case 各一次，最大预留 ¥1.90，不释放未知余额，也不据估算释放预留。缺少官方账单，不将预留说成实际扣费上界已核对。

2026-10-03 [官方标准价](https://platform.minimax.cn/docs/guides/pricing-paygo)：≤512k 输入每百万 ¥2.10，输出 ¥8.40。预留计算额外使用未折扣长上下文输入 ¥8.40／输出 ¥33.60，估算输入 allowance 8192 tokens＋输出硬上限 1024，约 ¥0.1033，再向上预留 ¥0.50。固定合成 request JSON ≤4096 bytes，64／256 output；不是供应商 tokenizer 的精确上界证明。

create_new admission 在 dispatch 前保存；旧 admission pending／失败／未知 usage 阻止新调用。当前只支持 text、tool 各一次，不能 CLI 自动补录或重试。未知/部分失败立即停下交付已得证据。初次认证非本 adapter、非流式，其 tokens 只记历史账本。

## 本步实现与待验收

- service-owned 两套流式契约，单 binding 直接派发；immutable revision，call／attempt／顺序号关联。
- MiniMax 内部完成 JSON/SSE 编解码；reqwest＋eventsource-stream，libs 不读 env/files；工具不执行。
- 手动入口白名单捕获，不先完整抓包。projected records 与 mapped events 分开，expected 独立审阅；replay 无网络／秘密装配。
- 不新增或运行自动化测试；静态 fmt/check/clippy 与人工 live/replay 分开记录。

仓库及工作区未发现适用 SKILL.md，不引入其他工作流。共享机器绝对路径不进入规范。
