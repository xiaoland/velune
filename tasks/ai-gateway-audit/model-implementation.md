# 模型业务重构与 hard-cutoff 验收

初次源码为 `8f15ae812b7bd0cec344b78b06d2eb8f2b9f4a9d`，该 clean 安装包在端到端验收中暴露严格列表契约遗漏，随后继续修复。配置 schema 4，显示版本仍为 0.1 beta.1。此次删除旧契约、迁移代码和重复的旧人工验收入口，不读取真实配置、秘密、会话或调用真实模型服务。

Model 表示可跨提供商关联的真实型号；application 自动生成隐藏 recordKey。ProviderModelBinding 保存该提供商精确的 providerModelId，以及可选上下文、输出能力和协议推理声明。未知规格不从另一个提供商或全局模型推导；Pi 所需数值在运行时准备边界检查。Route 选择 modelRecordKey／providerId，网关自动产生 `velune/model/<recordKey>`；裸记录键不是 HTTP 请求入口。网关完成唯一目标解析，原生协议 adapter 只校验 body.model 与 ProviderModelId 一致，不进行二次映射。

导入使用独立 candidateKey，显式关联只能指向已有模型记录，不按同名自动合并；替换同一来源绑定保留既有记录。Mac 表单不显示内部键，能力规格可空；详情 Disclosure 正文采用原生 Grid 与完整 leading 宽度，移除 LabeledContent 列宽造成的额外空白。没有进行 GUI 验收，布局体验留给用户。

配置只接受 schema 4。低版本普通配置原子重置为空 schema 4，不备份、不转换字段；原 Harness 文件、会话与 Keychain 秘密不删除。未来版本、损坏 JSON 和当前 schema 的旧字段明确拒绝且不改文件。

全 workspace `cargo fmt --all --check`、`cargo check --locked --workspace --all-targets --all-features` 和严格 clippy 通过；bindings 无 local-runtime 检查与严格 clippy 通过。全量 Swift app 与 credential helper 的 warnings-as-errors typecheck 通过，release 构建和 codesign deep strict 验证通过，已安装到 `/Applications/Velune.app`。

手动运行 [配置与认证脚本](../../scripts/manual-authentication-management.py)，portable 已通过 schema 重置／无备份、原 Harness 文件不变、自动记录键、同一模型两提供商不同 API 标识与独立能力，以及认证目标 guard、generation 冲突、精确清理和重开持久配置。它不调用凭据解析。未来 schema／损坏文件／旧字段的拒绝均确认文件不变。

手动运行 [原生 HTTP 脚本](../../scripts/manual-gateway-native.py)，9 个合成 ChatCompletions／Responses JSON、SSE、状态码、安全头、历史和扩展字段边界通过；缺少规格仍可调用，裸记录键拒绝且不派发上游。流前／流中断连传播和空闲 Runner 关闭通过。

初次安装库的配置脚本及 [正常 Pi 循环脚本](../../scripts/manual-pi-native-loop.py) 均在 list 边界拒绝 descriptor 多余 capability 字段。portable 没有运行时来源，未命中这条路径；静态检查无法核对内部 JSON 构造与严格类型之间的所有字段。修复将运行时与导入描述符改为具名类型构造，删除未消费字段，保留严格合同，避免恢复“忽略未知字段”来掩盖问题。

最终安装源码为 `6efbcc6d248f0673494985ea420fada8acd64b41`，manifest 为 clean／schema 4／0.1 beta.1。用该安装库重新运行配置与认证脚本通过全部边界；正常 Pi 循环通过配置运行时 → 导入 → 路由 → 连接 → 创建会话 → 消息 → 工具 → 续写 → 下一轮。共 3 次合成上游请求，真实固定 Pi SDK 与来源 credential adapter 参与执行；精确上游 model-id、原生推理历史与工具关联保留。未知显式模型映射拒绝且配置不变，跳过导入保留连接，替换导入清理旧连接，认证 generation 更新使旧预览失效。helper 在断开、约 30 秒期限及关闭时终止完整合成进程组，来源文件保持不变。

所有人工脚本使用临时 HOME、合成来源、回环上游与 fixture-only 认证，未接入 CI 或自动测试。没有读取真实配置／秘密／会话、调用真实模型服务或执行 GUI 验收；安装后没有自动启动应用。用户启动旧 schema 配置时将按已确认 hard-cutoff 行为重置普通配置。
