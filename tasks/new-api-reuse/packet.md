# RelayKit 网关接入

2026-10-11 用户希望复用 RelayKit 的 LLM 协议转换，参与生态并减少重复实现，同时核对 Go 接入与开源许可。初轮调查后，用户明确授权持续推进至完成接入或获得无法接入的具体证据，并允许自主提交。可考虑从相邻 Factory26 寻找真实提供商或重放入口；不输出、保存或提交真实凭据，不读取真实会话。没有远端发布或联系上游授权。

长期接入边界归 [AI service 设计](../../docs/design/ai-service.md)，本任务保存版本、许可调查和验收证据。既有转换验收见 [协议转换任务](../llm-protocol-translation/packet.md)。主执行者负责集成、文档和安装；唯一源码 owner 负责转换接入；advisor 评估工程边界。

## 已核对事实

New API 本轮查询 main 为 `0f38a14a5e8e322ea741478b0e49878be3130e1d`。其 [RelayKit README](https://github.com/QuantumNous/new-api/blob/0f38a14a5e8e322ea741478b0e49878be3130e1d/relaykit/README.md) 描述独立 Go 模块，覆盖 ChatCompletions、Responses、Messages、Gemini 的请求、非流式响应和流式事件；HTTP、SSE、鉴权、路由和计费由宿主承担。Go 最低版本为 1.25.1，模块处于 v0.x，公开 API 和 DTO 可能随小版本改变。其中当前三个已支持协议的六向转换已按下文实测；Gemini 不在本次接入范围。

RelayKit 明确沿用根 AGPLv3，不能因独立模块而当作宽松许可。[NOTICE](https://github.com/QuantumNous/new-api/blob/0f38a14a5e8e322ea741478b0e49878be3130e1d/NOTICE) 另列署名和界面链接要求。Velune 当前 Git 跟踪文件中未发现项目根许可证；产品许可选择不能由本次依赖建议代替。

Velune 现有 `packages/ai-provider` 拥有转换语义，`packages/gateway/src/translation.rs` 处理传输接线；application 拥有配置与认证来源。候选库替换不要求搬动整个 gateway。复用是否成立还需验证截断、终态、未知计量、工具关联、取消和跨语言状态释放，并检查日志及媒体回调边界。

## 已采用的工程边界

独立 advisor 与主执行者建议优先评估同进程 RelayKit，仅替换异协议转换。Rust 继续拥有 HTTP/SSE、认证、路由、取消与原始计量；平台 app 公开接口继续使用 UniFFI。Go 官方提供 c-archive／c-shared 构建模式，但这不是目标平台打包可行的实测证明。完整 New API 适合用作用户已有的独立上游，不建议作为当前产品必需后台；sidecar 仅在链接方案实测不合适后评估，也不能据此推定许可隔离。

建议最小后续实验使用临时 Mac 同进程桥接和合成材料，覆盖现有六向转换、工具参数分片、未知 usage、正常终态、截断、错误与释放；记录桥接规模、产物体积和打包步骤。若必须在桥接里重新维护大半套转换语义，复用收益不成立。采用后删除被替代实现，不长期维护两个引擎或隐式 fallback。当前已实现同进程桥接，具体结果见下节。

## Magpie 复核

调查者固定 Magpie `612b759d2141cccb4f477645c9d765c0878476b0`：[go.mod](https://github.com/yetone/magpie/blob/612b759d2141cccb4f477645c9d765c0878476b0/go.mod) 没有 new-api／RelayKit 模块依赖，[LICENSE](https://github.com/yetone/magpie/blob/612b759d2141cccb4f477645c9d765c0878476b0/LICENSE) 为 MIT。[README](https://github.com/yetone/magpie/blob/612b759d2141cccb4f477645c9d765c0878476b0/README.md) 的 New API 复用描述指社区渠道 middleware 的行为与 JSON 配置兼容，不能等同于直接使用转换库。未穷尽逐文件代码来源、历史或社区插件许可，因此也不能由无依赖推断绝无复制代码。

## 已授权实施范围

用户已确认所指组件为 RelayKit，授权技术接入及验证。最终发行许可方向正等待用户选择；本地开发继续，不擅自给整个 Velune 添加许可证。独立 advisor 确认第三方库私有 C ABI 不改变 App 的 UniFFI 产品接口，建议 Go c-archive 链入 Rust cdylib，以同步字节缓冲和请求范围句柄封装转换状态，不接管网络或凭据。

源码 owner 贯通 ai-provider 的 Go 桥接、Rust 替换及 gateway 必需接线；主执行者负责文档、许可打包、构建安装和端到端验收。调查 owner 只读寻找 Factory26 已有安全验证入口。先证明同进程装载与转换，再删除旧转换语义实现，随后执行 fmt/check/clippy、六向合成网关与外部客户端工具续轮、Mac 构建安装。禁止自动化测试、隐式 fallback 或长期双引擎。

## 当前实现与证据

固定模块版本 `v0.3.1-0.20261010122904-0f38a14a5e8e`，桥接与上游 Go module 最低版本均为 1.25.1，本机使用 Go 1.26.2。Go c-archive 已链接入 Rust bindings 动态库并成功装载；旧 Rust request／response／stream 转换器约 1912 行删除。同协议保持原生路径。Rust 唯一请求句柄被流转换消费，Drop 只释放、不生成终态。Go 不持有网络或凭据，原始 analytics 继续由 Rust 观察。

必要宿主适配包括省略跨提供商服务端状态引用、首 choice、未知 usage 字段保留未知、纯内联媒体解析和原生终态核对。RelayKit Responses 流的包装对象必须取 Payload 后序列化；直接编码包装对象会导致真实客户端不能消费，已修复。Android 使用同进程 c-shared，Apple／Windows／Linux 等有目标映射；仅 Mac 已实测，映射不等于 SDK、链接和打包已经验收。

人工直接桥接检查通过 custom／namespace JSON 与每三字符参数分片、连续 sequence、未知／部分 usage、截断、failed 响应、缺失必需终态、禁止外部媒体联网和 panic 固定无正文错误。仅使用临时脚本，无新增或运行自动化测试。

HTTP 人工脚本已改为消费者协议解析，检查工具 ID／名称／参数、最终文本和终态，支持交错工具与空 choices 的独立 usage。修正了旧 Responses 合成 fixture 同时在 added 与 delta 提供全文的不规范序列，没有为该 fixture 修改生产转换逻辑。真实 Codex 0.159.3 的 command／namespace × ChatCompletions／Messages 四闭环已通过；实际 Pi Agent 1.0.2 安装的 AI SDK 为 1.1.0，验收入口按真实安装解析，报告实际 SDK 版本。

另一项独立任务同时修改原工作区的 runtime/application 及依赖，最终验证转入仅包含本任务改动的隔离工作树，不混入其源码或提交。隔离 fmt、cargo check --locked、workspace/all-targets/all-features clippy -D warnings、bindings no-default-features check/clippy 均通过。六向 JSON／SSE 29 次合成请求通过，包含两轮工具、分片、错误、取消、截断和缺省上限。实际 release bundle 的 Codex 四闭环与 Pi AI SDK 1.1.0 闭环合计 10 次上游请求通过。同协议 9 条保真路径、17 路并发、取消和关闭回归通过；最新 usage 字段清理后六向 29 次再次通过，Go vet 通过。Mac release 构建、Swift 自有目标 warnings-as-errors 和签名校验通过；源码提交 `ff2aa83` 已从干净隔离工作树重建并安装至 `/Applications/Velune.app`（0.1 beta.1），安装包 manifest 的 source_commit 一致且 dirty=false。安装包内动态库再次通过 Codex 四闭环与 Pi SDK 闭环，共 10 次合成上游请求；安装签名校验通过。

Factory26 仅只读检查公开部署说明、模型目录与脚本，未读取 endpoint/key 私有值或真实会话，未发现可直接使用的脱敏协议重放。未调用真实提供商。安装包保留固定 RelayKit、Go runtime 及七个实际运行时依赖许可证；七个许可证已与固定模块缓存核对完整正文，仅规范化 sjson 许可证末尾空行。最终公开发行许可方向仍待用户确认，本地接入不擅自给整个项目添加许可证。

## 完成状态

技术接入、删除旧转换器、静态检查、人工闭环、Mac 打包安装均已完成。公开发行尚未授权；AGPLv3 及上游 NOTICE 的最终发行安排仍待产品许可决定，不影响已完成的本地验证。未验证其他平台实际构建，未调用真实模型。
