# quotio 参考与额度借鉴

2026-10-27 用户希望参考 quotio 改进 AI 提供商、模型路由与聚合，先只看不照搬；比较后同意借鉴前四项（[S55](../../docs/sources.md#s55)）。产品行为归 [模型路由](../../docs/prd/model-routing.md#调度额度与使用上限) 与 [AI 提供商](../../docs/prd/ai-providers.md#账户额度历史与提醒)，契约归 [AI 服务设计](../../docs/design/ai-service.md#路由fail-over-与生命周期)、[gateway](../../packages/gateway/README.md) 与 [bindings](../../packages/bindings/README.md)。

## 比较

源码为 [nguyenphutrong/quotio](https://github.com/nguyenphutrong/quotio) 提交 `210cdb0`（2026-10-23），只读临时 checkout，未运行。quotio 是 macOS 菜单栏应用加 Rust CLI，核心是约 50 个提供商的额度可见性（窗口、重置、历史、提醒）；请求聚合交给外部 CLIProxyAPI，自身只透传轮询／填满策略与重试设置。Velune 的路由（四种策略、黏性、冷却单探测、响应提交后不切换）已更完整，额度一侧 quotio 更广。

借鉴的四项：

1. 按重置节奏排序：quotio 的 `QuotaPace` 以均匀消耗为基线判断是否会提前用尽。Velune 原先只比较余量，并丢弃了 Codex 报告的窗口时长。
2. 金额：quotio 的额度可为数额与单位。Velune 原先只有比例窗口，余额类提供商无从表达。
3. API key 提供商的账户额度：quotio 的目录给出多家接口；Velune 只接入有公开文档的 DeepSeek `GET /user/balance`、Moonshot `GET /v1/users/me/balance` 与 OpenRouter `GET /api/v1/key`。智谱／Z.ai `quota/limit` 与 MiniMax `token_plan/remains` 只见于社区插件，字段含义有公开的误标问题；阿里云百炼、豆包等只在控制台提供；均暂不接入。
4. 历史与提醒：沿用 quotio 时间线规格的原则：只记成功观测、不补点、空白不连线、认证改变时另起记录、保留 30 天。提醒复用既有使用上限，不另设阈值；不为历史增加后台读取。

未借鉴：CLIProxyAPI 以 OAuth 登录各家后暴露为 API，即借 CLI token 冒充正式接入；quotio 的 Claude／Codex 额度读取取用 Claude Code Keychain 令牌或 `~/.codex/auth.json`；定时预热请求以提前开启额度窗口，接近规避供应商限制并产生实际调用。第五项（本机登录发现的安全规则）未在本轮授权内。

## 实现

- gateway：`QuotaWindow.window_ms`；smart 以余量除以周期内尚余时间比例（下限 0.01）排序。
- application-api／application：状态带节奏与金额；`ApiAccount` 额度来源只接受精确官方主机，配置校验拒绝其他组合；新增三个 API 模板；历史 JSONL 与提醒序列；`quotaHistory`／`clearQuotaHistory`／`quotaAlerts` 命令与远控转发。
- Mac：窗口节奏提示、金额行、额度历史 sheet、账户额度开关、提醒通知。
- Windows：智能策略说明、金额文字、OpenRouter 窗口名；历史、通知、节奏提示与开关记入 [PARITY](../../app/windows/PARITY.md)。

## 验证

均使用合成数据，没有调用真实供应商或读取真实 Key。

- Rust：workspace check、全 targets 严格 clippy、bindings `--no-default-features` 检查通过。临时测试覆盖官方主机边界、三家响应解析、OpenRouter 日／周／月周期、节奏、提醒去重、历史读写／分段／保留与 gateway 节奏排序，全部通过后删除，只保留主机边界测试 `only_exact_official_origins`（通过）。
- Mac：warnings-as-errors 构建通过。临时离屏渲染（未提交）以合成状态检查 Codex 额度页（节奏提示、上限、历史按钮）、DeepSeek 余额页（无上限节）、未报告额度的 OpenRouter、24 小时历史图（休眠空白）、DeepSeek 编辑器开关与中转地址无开关；据此修正了历史图的条形、坐标标签截断、只有金额时的上限节和开关位置。
- Windows：在 `win-ws.localhost` 的独立目录 `C:\Users\yyh\velune-quota-borrow`（源码为 `97b69b6` 的归档）完整运行 `scripts\build-windows.ps1`：Rust Release、C# 绑定生成与 WinUI publish 均通过，日志无 C# 错误或警告。未运行应用、未做界面目视检查。
- 落地：两个提交 rebase 到 main `0cee555` 后，重跑 workspace 与 no-default 严格 clippy、保留测试和 rustfmt 检查，并以 `97b69b6` 签名构建、安装 `/Applications/Velune.app`（manifest 非 dirty），随后快进 main。

未验收：真实 DeepSeek／Moonshot／OpenRouter Key 的余额读取、OpenRouter 有上限 Key 的周期、通知横幅的实际呈现，留给用户使用时确认。
