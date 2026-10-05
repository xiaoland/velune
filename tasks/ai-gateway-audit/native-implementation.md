# LLM Gateway 原生执行：实施与人工验收

2026-10-05，用户在职责复核后授权持续迭代实施。产品源码提交为 `22a15c0`；本地 Mac bundle 已重新构建、签名验证并安装，manifest 对应该提交且 `dirty: false`，显示版本仍为 `0.1 beta.1`。本记录只覆盖合成、隔离的功能边界，不替代用户的 UI、真实认证或真实服务验收。

## 本轮实现

ChatCompletions 与 Responses 按协议执行，LLM Gateway 不再使用 sampling 往返转换或 MiniMax mapper。只改写路由模型标识和应用上游认证，保留原生请求字段、历史、JSON／SSE、HTTP 状态及安全请求／响应头。不隐式加入 token 上限、推理参数或 usage 选项。业务 Usage／Quantity 归 sampling 数据；观察可消费它们，旧单操作服务改名为 SamplingService，不代表整个 AI 模块。

Axum 接管 HTTP framing，eventsource-stream 观察 Responses SSE。网关与 provider 的事件交付可等待，容量为 1 的通道保持背压；下游 body 丢弃会取消派发，不拼接或合成响应流。Responses 的 completed、incomplete、failed、cancelled 是业务终态，完整传输不因业务失败被伪造为断流。单次认证捕获后交给 provider，HTTP client 禁用隐式重试和重定向。

Unix 凭据 helper 在独立进程组运行，具有 30 秒期限和 64 KiB 输出限制；断连、超时与退出终止整组并回收直接 helper。请求 body 限制为 256 KiB，非流正文限制为 16 MiB，Responses 待解析事件输入预算为 256 KiB；最多 16 个已进入 handler 的请求。Header Debug 隐藏全部值，Payload Debug 不记录正文。

Pi adapter 保留固定 SDK 1.0.2 从原 provider／URL 推断的有效兼容设置、输入能力和采样参数。DeepSeek thinking 格式及 assistant reasoning_content 由 SDK 原生编码与解析；模型声明推理能力不改变现有 off 默认。订阅投影要求显式 OAuth 类型，API-key 来源不再套用订阅限制。只导入明确选中的模型及其提供商；相关网关导入实际发生变化后停止旧连接，重复跳过不停止连接。

## 验收证据

静态检查通过 `cargo fmt --all --check`、`cargo check --locked`、`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`，以及 bindings 的 `--no-default-features` 检查。JavaScript 和构建脚本语法检查通过。Mac release、UniFFI Swift 生成／编译、原生 app 与 helper 构建、bundle 严格签名验证通过。不新增或运行自动化测试。

[原生边界脚本](../../scripts/manual-gateway-native.py)以当前源码构建的 debug 库启动真实本机入口，9 个合成 HTTP 案例全部通过：Chat／Responses JSON、SSE、错误正文、chunked 请求，以及 Responses incomplete JSON／failed SSE。多个 system／developer 指令、推理历史、未知字段和省略输出上限均保留；正文逐字节核对，201／429、Retry-After、请求 ID 与自定义非敏感响应头保留，Cookie／Set-Cookie 不转发。响应前与流中断连均在 3 秒内确认上游关闭。挂住的 shell→Node helper 分别经过下游断连、30 秒超时和 Runner Drop，Node PID 均消失；idle 与挂住 helper 的 Drop 实测为 0 毫秒。完整人工脚本退出 0，临时目录清理。

[正常 Pi 循环脚本](../../scripts/manual-pi-native-loop.py)加载已安装 bundle 的 dylib 与同版生成 Python 绑定，通过具名应用用例完成：配置运行时 → 预览与导入 → 配置模型路由 → 连接 → 创建会话 → 发送 → 本地 read 工具 → SDK 自动续接 → 下一轮用户消息。共 3 次合成上游请求，全部通过 LLM Gateway。预览同时允许 reasoning 和 plain 模型的原生兼容设置；仅所选提供商被保存。tool call 关联和 reasoning_content 历史保留，最终回复进入会话 projection。重复导入跳过后连接保留；增加同一提供商模型并替换后返回 requiresReconnect 且旧活动连接清除。三份原来源文件字节不变。脚本退出 0，应用 shutdown 完成，临时目录删除。

人工运行入口如下，路径由执行者显式选择，不读取用户已有配置：

```sh
python3 scripts/manual-gateway-native.py \
  --deps "$PWD/target/debug/deps" --rustc "$(rustup which rustc)" \
  --node /absolute/path/to/node
python3 scripts/manual-pi-native-loop.py \
  --bundle /Applications/Velune.app \
  --bindings "$PWD/target/bindings/native-python" --node /absolute/path/to/node
```

Python 绑定由同版 release dylib 使用 velune-bindgen 生成。两个脚本只作显式人工验收，不接入 CI。旧 DeepSeek 诊断脚本（历史入口 `manual-pi-deepseek-import.py`，现已随 hard-cutoff 删除）针对 `7345ba8` 失败基线，不适用于当前 Runner API；保留历史记录只为解释原审计证据，脚本本身已删除。

## 当前限制

AI 服务不限于 LLM，当前 LLM Gateway 只是其一种应用模式；本轮不新建非 LLM 能力或通用网关。仅显式静态路由，fail-over Disabled，候选范围及不确定提交的重放策略尚未确认。Responses 只覆盖 foreground POST 创建，Anthropic Messages 等协议未实现。

持久配置与运行配置仍通过重连生效，未交付无中断热配置或统一版本生效机制。网关仍由当前本地运行时用例装配，独立 AI 能力本身无需经过网关；尚未新增独立网关运行用例。普通配置编辑影响无关运行时等原审计项仍需后续迭代，不能以导入失效修复声称全部生命周期问题已解决。

旧导入绑定缺少新增执行元数据时，需要重新预览、导入并明确选择替换；不会静默吸收来源变化。来源自定义头、动态凭据命令、未接入的 Responses 兼容选项仍有明确限制。Windows helper 等待 Job／进程树管理实现，不把 Unix 验收提升为 Windows 支持。原始 TCP 连接数上限、慢读者长时间 RSS、超大正文及超大未完成事件的动态故障注入未验收；资源预算经过类型／控制流检查，不冒充全部运行证明。没有真实登录、真实 API 或用户 UI 操作。
