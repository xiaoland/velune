# AI service：有界 MiniMax 流式实现与 fixture 验收

- 基线：`7c6f98267a56101d2117311adeaf10b50b01d8f2`（dev/ai-service-contracts），新分支 `dev/minimax-stream-fixtures`
- 设计权威：[AI service](../../docs/design/ai-service.md)，用户追加授权优先于此前仅类型范围
- 授权：最小直接派发 service、MiniMax adapter、合成文本／工具流真实 fixture、手动离线重复回放；普通 push 新 dev 分支
- 排除：新测试／框架、cargo test、路由／fallback 算法、Ark／Bailian、Host／UI／Mac／部署、PR／main／force
- 状态：实现已提交；静态检查通过。文本 live 返回 200／finish／usage，但未解析到 DONE，当前映射按 partial Transport 终止；失败样本离线重复验收通过。工具 live 未执行，完整目标未验收。

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


## 实际证据（2026-10-03 UTC）

采集源码 commit：`abd05a2cea3f0a7545005778c030bc78b5b9890f`，基于指定基线；采集时工作树干净。此后仅修正 replay 对 source HTTP 200 的 execution knowledge 保留，未改写 live 文件。

| 项目 | 实际结果 |
| --- | --- |
| 静态 | Rust 1.99.0；fmt/check/clippy（两个 lib＋manual example）通过；没有 cargo test／测试框架 |
| 文本 live | 本轮新增 1 HTTP attempt，200；模型报告 MiniMax-M3；片段 `S`、`YNTHETIC_OK`；finish stop；usage 171 input／7 output／178 total |
| 终态 | 未解析到 DONE；Transport、Accepted、partial text=SYNTHETIC_OK；不是成功 sampling 验收 |
| 独立 expected | 人工从五个 retained chunks 审阅：2 text＋finish＋usage＋terminal，共 5 service events；未由 Decoder 自动生成 expected |
| 离线 repeat | replay text 连续两次一致；每次 network_attempts=0、credential_reads=0；失败／partial 与 reported usage 原样重现 |
| 工具 live／replay | 未执行；触发 partial 停止条件后没有继续计费采集，不以手写工具样例冒充真实 fixture |

文件：[live](../../fixtures/ai/minimax-m3/text.live.json)、[expected](../../fixtures/ai/minimax-m3/text.expected.json)。capture_unix_seconds 记录真实采集时钟，协议／requested model／响应 model／source commit／revision／call／attempt／脱敏规则／source usage presence 均保留。额外 usage 字段存在但未映射，其值未捕获。

### 明确阻塞与安全接续

当前 adapter 假设必须有 SSE `[DONE]` 才完成；官方 MiniMax 文档确认 streaming／finish／usage 参数，但本轮没有找到必须发送该哨兵的明确承诺。现有白名单采集只记录完成的 SSE data events，未区分 `EventStream` 错误与 clean EOF，也不保存未派发尾帧。因此**不能据此认定供应商断流，也不能据内容完整就伪造成功**。

后续需要先增加无敏感值的 EOF／typed parser/transport 分类观测，并核对 MiniMax 的实际终止契约；不得靠添加 DONE 到 fixture 或放宽成任意 EOF 成功来消除失败。工具分片／ID／参数组装有代码与静态证据，尚无真实工具 fixture 或运行验收。采集停止后未重试、未 fallback、未追加模型调用。

### 费用账本

按官方当前标准价、不扣缓存折扣的估算：

- 先前认证 165 input＋2 output：¥0.0003633（usage 已报告，金额 Estimated）
- 本次文本 171 input＋7 output：¥0.0004179（usage 已报告，金额 Estimated）
- 已知两次合计 336 input＋9 output：¥0.0007812
- 历史 literal-name 401：usage／实际扣费仍 Unknown，未归零
- 当前保守占用：历史 ¥0.80＋认证 ¥0.10＋文本 ¥0.50 = ¥1.40；总上限 ¥5。此为人工预留账本，不是已查询账单或精确剩余额度。

本轮新增 attempt=1；跨已知历史共 3 次（401、认证成功、文本流）。工具 0，Ark 0，Bailian 0。没有查询余额／账单／模型列表，未购买额度。


## 同轮诊断与修复（继续授权）

用户要求在剩余授权预算内继续恢复问题，允许至多一次诊断 capture，再完成工具流；不新增测试／框架。本段取代上段“需要后续诊断”的阻塞状态，但保留旧样本事实。

- 零模型请求检查 eventsource-stream 0.2.3 源码：EOF 不派发未闭合 data，原代码又将 parser／transport／clean EOF 统一为 Transport，因此旧 fixture 本身不足以区分原因。
- 诊断源码 `d7439bdd67b6a2810d25e8f7e16c6632011eab99`，增加字节级无正文 framing observer、typed termination，以及写入前解码／重组内容的 credential reflection guard。
- 唯一诊断新增 1 HTTP attempt，200，171 input＋7 output。真实记录 `CleanEofWithoutDone`，done_lines=0、done_frames=0、unfinished_line=false、unfinished_data_frame=false。不存在吞尾 DONE、未闭合帧或 transport/parser error 的证据。
- root cause：本 adapter 擅自把 DONE 作为唯一完成信号。MiniMax 官方 [SDK 流式示例](https://platform.minimax.cn/docs/api-reference/text-openai-api) 使用自然迭代结束；[官方 OpenAI SDK 实现](https://github.com/openai/openai-python/blob/main/src/openai/_streaming.py) 允许 DONE 或正常底层迭代结束，没有“无 DONE 必定失败”契约。这个源码是检查日参考，非本仓库依赖。
- 修正要求同时满足：clean transport EOF、没有未闭合 SSE 行／帧、没有未派发 DONE、显式合法 finish、input/output usage 已报告、完整合法文本／工具结果。不会在 EOF 错误／未知 usage／半截参数时伪造成功。
- [diagnostic live](../../fixtures/ai/minimax-m3/text-diagnostic.live.json) 保留 capture 时失败 mapped，不改原数据；[diagnostic expected](../../fixtures/ai/minimax-m3/text-diagnostic.expected.json) 独立审阅后明确采用新终止规则。修正后 replay 连续两次成功映射 5 events；原 text 仍按缺少终止证据保留失败 replay。
- 修正 replay 错误归因：Decoder 的 Unsupported／ProviderFailure 等不再统一变成 ProviderFailure；HTTP 200 后失败保留 Accepted，HTTP 非 200 保留 Unknown；replay 本身网络 submissions=0。
- 手动 live 入口只增 text-diagnostic 一个固定 admission，旧文件不覆盖；tool 需诊断 expected 与实际 replay 一致才可派发。最多三个本步 admissions，预留最大 ¥2.40，仍少于授权 8 次／¥5。

当前新增 attempts=2，已知本阶段 342 input＋14 output；加认证合计 507 input＋16 output，标准价估算 ¥0.0011991。历史 401 Unknown 不变，保守预留当前 ¥1.90。尚未发送工具请求。
