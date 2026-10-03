# AI service 契约与配置有界实现

- 输入：[S13](../../docs/sources.md#s13)、[权威设计](../../docs/design/ai-service.md)
- 基线：已发布 `3bce5f975f0e65dddc2185faa377329282989a92`
- 范围：独立 AI lib 两套契约、provider 配置、必要类型与运行时构造校验、文档和静态检查
- 排除：路由／fallback／重试、网络 provider、凭据读取、UI／Host 改造、新测试、既有集成测试运行
- 状态：契约与校验已写入，静态检查通过；接口审阅待父会话／用户确认，运行不变量未验证

## 工作隔离

暂停的 IPC／测试及文档改动原样留在原工作树，没有 stash、删除、提交或回滚。新 worktree 的 `dev/ai-service-contracts` 从上述发布基线创建；本轮修改不复制暂停 diff。工作区机器路径只在交付消息说明，不写为共享规则。

原工作树未提交 diff 的隔离前 SHA-256：`e0af5a9bfa0b3412f21b3144a84ab1ebea0f998a58bb20202ab19b10f149d476`。交付前再次核对，证明本轮未改暂停内容。

## 审阅验收

1. 实际公开接口：AiService::sampling 是调用方入口；SamplingProvider 是 service 所有的 inward 契约。provider 配置 crate 单向依赖 service，无 SDK 类型穿透；不存在平级 sampling 执行层。
2. ProviderConfig 只含协议／凭据引用／模型列表及 ID／revision；构造器拒绝无效端点、空模型、重复 ModelId 等。service 不读取／保存配置，app 装配职责只写契约，未实现配置中心。
3. ID newtype 不互换；请求 target 必须属于捕获 binding。PreparedSampling 不暴露变更接口；Arc 仅保证对象身份，真实实现不重读配置仍需运行验收。
4. 工具定义／调用／结果有结构化关联但不执行；未知用量不默认零、reported 和 estimated 不混淆；失败可保留部分结果及用量；观测分开逻辑 call／实际 attempt，不默认携带正文。
5. 阅读 Cargo diff、代码及静态检查结果后由父会话审阅；编译通过不是本步全部验收通过。没有执行 provider、真实 service、并发热更新或真实模型效果证据。

## 后续最小真实接入与 fixture 验收方案（尚未启动）

用户已提出真实接入、捕获 fixture 便于重复验收；当前等待首个 provider／model、已配置凭据位置及额度／费用上限。未回复前不发请求、不读凭据、不自行选供应商。普通固定响应 adapter 示例方案亦未执行，不增加测试框架。

收到这些条件后再限定一次采集的范围：

- 使用公开／合成文本和工具 schema，事先固定 request 与验收目标；禁用外部动作型工具，不提供用户代码、私人会话或秘密。
- 捕获源必须区分 `live` 与 `replay`，记录 adapter／协议版本、配置 revision、请求模型名、响应报告模型名（若有）、采集时间及实际 attempt 关联。模型别名和返回型号不一致时保留两者，不事后改写。
- raw fixture 是**脱敏协议记录**而非完整网络抓包：只白名单保留请求 JSON 和协议事件／字段；不保存 auth headers、token、cookies、账户／组织标识或私人内容。必要删除字段用 redaction manifest 说明，保留事件顺序和已知结构；采集后先审阅再入库，禁止把含秘密的原始包暂存进 Git。
- 把采集协议映射到 SamplingInput／Output／Failure／Usage／AttemptContext，给出每个期望字段的源字段和规则。当前返回完整结果的契约没有流式事件 API；若首个 provider 需要流事件，先限定其服务契约映射，不在 adapter 外复制协议转换层。
- 随机响应内容本身不是 oracle。人工批准的验收断言只覆盖已确认不变量：请求／结果关联、工具 ID／参数结构、终止／错误语义、usage reported／estimated／unknown、一次逻辑调用与真实 attempts、正文不进入默认观测。不得为了通过而改写捕获输出或把缺失 usage 填 0。
- 回放只说明同一 fixture 对契约映射可复现，不代表实时网络、当前模型能力、凭据／权益或成本再次验证。固定响应手动调用／fixture replay 工具和真实 adapter 均需下一步明确实现范围，不自动变成新测试平台。

完成当前有界交付后停止，等待审阅与上述外部条件；不启动 Mac 或下一阶段。

## 本步静态证据

2026-10-03，Rust 1.99.0，以下命令均通过：

```sh
cargo fmt --all --check
cargo check --locked --offline -p velune-ai -p velune-ai-provider --lib
cargo clippy --locked --offline -p velune-ai -p velune-ai-provider --lib -- -D warnings
cargo tree --locked --offline -p velune-ai-provider
```

依赖树为 `velune-ai-provider → velune-ai → serde_json`，复用已锁版本，没有新增外部包／SDK。Cargo.lock 只增加两个本地 package。未新增或运行任何测试，未运行示例、Host、UI、Mac 构建或真实 provider。

回读差异确认：原 `src/`、`tests/`、`native/`、`scripts/` 无变更。原工作树暂停 diff 的 SHA-256 与隔离前一致。新代码的静态保护和构造器实现可审阅，但本轮没有构造器运行结果／真实调用证据；验收边界按上表保持未完成项，不冒充闭环。
