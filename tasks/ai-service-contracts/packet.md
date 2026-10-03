# AI service：有界 MiniMax 流式实现与 fixture 验收

- 基线：`7c6f98267a56101d2117311adeaf10b50b01d8f2`（dev/ai-service-contracts）；交付分支 `dev/minimax-stream-fixtures`
- 设计权威：[AI service](../../docs/design/ai-service.md)；授权来源 [S14](../../docs/sources.md#s14)
- 完成：直接流式派发 service、MiniMax adapter、真实合成文本／工具记录、独立 expected、手动离线重复验收、静态检查
- 状态：文本诊断定位并修复错误的 DONE-only 完成条件；工具 live 在修正后成功。文本修正通过真实诊断样本 replay 验证，没有再发文本请求。原两份失败 capture 的 mapped 数据未改写。
- 排除：新测试／框架、cargo test、自动路由／fallback、Ark／Bailian、Host／UI／Mac／部署、PR／main／force

## 架构与装配

service 拥有 caller／provider 两套权威流式契约，sampling 是通用 AI lib 的操作。DirectAiService 同步捕获 immutable binding 后返回 Future，单次直接派发；provider 自己处理 HTTP／SSE／厂商 JSON。统一配置由 app main 装配；lib 不读 env/files，没有账户／渠道实体。call／attempt／revision／sequence 分开，Usage 区分 Unknown／Reported／Estimated。工具只返回、不执行。

手动例程是本步 composition root，不是正式配置中心。复用当前 Cloud 的 Networksecret；只在内存消费 MINIMAX_API_KEY 占位，标准环境代理、系统 TLS 校验，固定 `https://api.minimax.cn/v1/chat/completions`／MiniMax-M3 普通 API。thinking disabled、standard 服务层、无内置收费工具，retry／redirect 禁用。

Rust 1.99.0 官方 minimal 工具链，本地安装且未改 PATH 配置或安全策略。reqwest 0.13.5、eventsource-stream 0.2.3 锁定 Cargo.lock；新增 99 个依赖 package 版本，既有 package 版本未变。没有厂商 SDK 或应用大框架。仓库／工作区未发现适用 SKILL.md。

## Root cause 与完成规则

1. 原文本 capture 收到 HTTP 200、两段完整文本、stop、usage，却没有 parsed DONE；原实现要求 DONE，返回 Transport＋Accepted＋partial。
2. 零模型请求检查 eventsource-stream 源码和官方文档，确认旧记录丢失 EOF／parser／transport 分类，不能推断网络失败。
3. 唯一一次诊断新增字节级无正文 observer：实际 `CleanEofWithoutDone`；done_lines=0、done_frames=0，unfinished_line=false、unfinished_data_frame=false、unfinished_line_is_done=false。没有未派发 DONE、尾部未闭合或 transport/parser error。工具 live 后来复现相同自然结束方式。
4. root cause 是 adapter 把 DONE 当唯一终止契约。[MiniMax 官方 SDK 流式示例](https://platform.minimax.cn/docs/api-reference/text-openai-api) 使用迭代结束；检查日 [OpenAI 官方 SDK 实现](https://github.com/openai/openai-python/blob/main/src/openai/_streaming.py) 也接受正常迭代耗尽，而非断言一定收到 DONE。SDK 源码仅参考，非本项目依赖。
5. 修正后的完成条件：已解码 DONE，或 **clean transport EOF＋无未闭合 SSE 行／帧＋无被漏解析的 DONE＋显式合法 finish＋input/output Reported usage**；两种路径都校验完整合法输出／工具参数。没有凭任意 EOF、仅 finish 或仅文字完整就宣告成功。

Decoder error kind 在 replay 中原样保留；HTTP 200 后失败为 Accepted，非 200 为 Unknown，preflight 为 NotSent；replay 的实际网络 submissions 始终为 0。原 text 没有终止证据，依旧按原失败 expected 回放，不追补／伪造其 EOF。

## 真实 capture 与独立 expected

| Case | Capture 源码 | HTTP／终态 | 报告 tokens | 回放验收 |
| --- | --- | --- | --- | --- |
| [text](../../fixtures/ai/minimax-m3/text.live.json) | `abd05a2cea3f0a7545005778c030bc78b5b9890f` | 200；旧 DONE-only 映射失败，保留 partial | 171 input／7 output／178 total | [expected](../../fixtures/ai/minimax-m3/text.expected.json)：原失败与 5 events 重现 |
| [text-diagnostic](../../fixtures/ai/minimax-m3/text-diagnostic.live.json) | `d7439bdd67b6a2810d25e8f7e16c6632011eab99` | 200；真实 clean EOF；capture 时旧 mapped 失败不改写 | 171／7／178 | [expected](../../fixtures/ai/minimax-m3/text-diagnostic.expected.json)：按修正规则成功，5 events，连续两次一致 |
| [tool](../../fixtures/ai/minimax-m3/tool.live.json) | `42e44010f5d99ae161e46cd85e41651c774fd065` | 200；修正后 live 成功，ToolCalls、clean EOF | 440／34／474 | [expected](../../fixtures/ai/minimax-m3/tool.expected.json)：5 events，连续两次一致 |

文本事实：`S`、`YNTHETIC_OK` 两片段，finish stop。工具事实：index=0，一次 add_numbers，参数 `{"a":2,"b":3}`，finish tool_calls；原始 tool-call ID 在 fixture 内保持一致，未执行工具。实际工具参数只有**一个片段**，不声称多片段／并行工具交错验收通过。

expected 由 retained source chunks／termination 事实独立人工审阅写成，不通过 Decoder 自动更新。所有 service events 的 call／attempt／provider／model／revision／sequence 核对；usage Reported 保持原值，total 留源记录，额外 usage 字段只保留存在性，未映射缓存计量。

source=live／replay 区分，capture 记录 protocol、model、source commit、实际时钟、config revision、call／attempt、脱敏清单与 source usage presence。采集时对应源码均已提交。replay 在入口分支不构造网络 client，不读取凭据；不重新验证网络或原始帧解析，仅重放源协议投影与记录的终止事实。真实 framing 证据来自诊断与工具 live 的字节层 observer。

## 隐私与失败边界

只白名单捕获必要 JSON／SSE data，不保存 headers、cookies、响应个人标识、auth、raw errors 或完整网络包。callbacks 仅在内存缓冲；写文件前检查所有解码字符串、拼接文本、按 index 重组工具参数及其内嵌 JSON，拒绝凭据／占位回显，包括 JSON 转义与跨片段拼接。失败时不输出值／长度／hash，不格式化异常，也不写正文。已保存文件的禁用字段名检查无命中。

此 guard 是已知注入值的内容检查，不声称识别代理端未知 raw secret、任意编码或恶意其他调用方。回显攻击变体未新增测试或真实请求；源码审核与正常 capture 经过 guard 不等于完整安全证明。Payload Debug 脱敏，观测无正文；显式访问和人工 fixture 仍属于授权内容面。

没有隐藏 retry／redirect／fallback。partial／unknown usage 停止新派发并保留预留；本次已获明确继续诊断授权，只对两份特定历史源码的已知终止错误允许有界恢复，且工具 admission 先离线比对独立 diagnostic expected。不得删除 admission 重复计费采集。

## 静态与手动证据

2026-10-03，以下检查通过，未运行 cargo test 或新增自动化测试：

```sh
cargo fmt --all --check
cargo check --locked --offline -p velune-ai -p velune-ai-provider --lib --example minimax_manual
cargo clippy --locked --offline -p velune-ai -p velune-ai-provider --lib --example minimax_manual -- -D warnings
```

手动 `replay text-diagnostic`、`replay tool` 各连续两次：mapping_matches_reviewed_expected=true，events=5，network_attempts=0，credential_reads=0；原 `replay text` 仍重现失败。相对文档链接与 Git whitespace 检查通过；旧 src/tests/native/scripts 路径无改动。运行命令归属 [开发说明](../../docs/development.md#独立-ai-service-的有界人工验收)。

## 预算账本与实际请求

用户总上限 ¥5，最多新增 8 HTTP attempts；最终本轮新增 **3**（初始文本、唯一诊断、工具各 1），加此前认证 1 和历史 literal-name 401 共已知 **5** 次。无 Ark/Bailian／余额／账单／模型列表请求，无购买额度。

2026-10-03 [官方标准价](https://platform.minimax.cn/docs/guides/pricing-paygo)：≤512k input ¥2.10／百万、output ¥8.40／百万。以下不扣缓存优惠，是基于 reported tokens 的 Estimated 金额，并非账单：

| 请求 | input | output | 估算 CNY |
| --- | ---: | ---: | ---: |
| 先前认证 | 165 | 2 | 0.0003633 |
| 初始文本 | 171 | 7 | 0.0004179 |
| 文本诊断 | 171 | 7 | 0.0004179 |
| 工具 | 440 | 34 | 0.0012096 |
| 已知合计 | 947 | 50 | 0.0024087 |

历史 401 usage／实际扣费仍 **Unknown**，未归零。保守预留：历史 ¥0.80＋认证 ¥0.10＋本步 3×¥0.50＝**¥2.40**，未按估算释放。预留使用高于当前标准价的 input ¥8.40／百万、output ¥33.60／百万，估算输入 allowance 8192＋输出硬限 1024约 ¥0.1033，向上预留 ¥0.50。实际固定 request JSON ≤4096 bytes，text 输出限 64、tool 限 256。输入 allowance 不是 tokenizer 精确证明，预留不代表实际账单或余额已核对。

当前入口只有 text、text-diagnostic、tool 三个 create_new admissions，每个最多一次；pending／未知不释放。它是人工有界验收护栏，不是跨进程／跨目录全局预算服务。

## 剩余未验收

并发热更新／凭据轮换；取消、drop／panic 后终态；真实断流／429／超时；多工具、参数多片段与交错；工具结果往返；usage 扩展字段无损映射；完整 replay 故障矩阵和秘密反射攻击矩阵；全局预算协调与账单；正式配置中心；三 Harness／其他 provider／Mac。到此停止扩展。
