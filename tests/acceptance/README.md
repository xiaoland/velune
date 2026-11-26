# 验收套件

可重放的验收用例：通过生成的 Python 绑定驱动实际 application，在私有 HOME 与 `VELUNE_HOME` 中复现用户路径。方针与分层见[验证方式](../../docs/development/validation.md#验收套件)。

## 运行

```sh
# 构建宿主机绑定库并生成匹配的 Python 模块，然后运行全部用例
python3 tests/acceptance/run.py --build
# 复用已生成的绑定，只运行一个用例，并传入实际 Codex CLI
python3 tests/acceptance/run.py codex_conversation_lifecycle --codex /absolute/path/to/codex
# 用例按 REQUIRES 声明所需 CLI（--codex、--claude、--pi），未提供的跳过
```

同一入口也注册为 `svc run acceptance`。缺少外部程序的用例记为跳过而非通过。默认绑定目录为 `target/acceptance/bindings`；`--bindings` 可指向另一次构建生成的目录，用于比较两个版本。`--resources` 指定某次构建自带的运行时资源，缺省时使用源码中的 helper 脚本与 `target/runtime-support` 的依赖。

每次运行在 `target/verification/acceptance/<run>/` 写入 `manifest.json`，记录源码 HEAD 与是否有未提交改动、绑定库摘要、外部程序路径与版本，以及逐个用例的状态、已证明的检查项、耗时和错误。每个用例另有完整日志与结果文件。任务包引用这份清单，不复制日志。

## 编写用例

用例是 `cases/` 下的一个模块，声明 `REQUIRES`（所需的外部程序，例如 `("codex",)`）并实现 `run(ctx)`。runner 为每个用例启动独立进程，先进入隔离环境：HOME、状态与项目目录均为私有临时目录，代理指向关闭的端口，只有回环地址可达。

- `ctx.bindings`：本次加载的 `bindings`、`application_api`、`conversation` 模块。
- `ctx.isolated`：`home`、`state`、`project`、`resources` 路径，`dir(name)` 创建附加目录。
- `ctx.tool(name)`：外部程序的绝对路径；未提供时用例被跳过。
- `ctx.passed(check)`：按顺序记录一项已证明的用户可见保证；`ctx.facts` 记录附加观测值。
- `velune_acceptance.responses.ResponsesProvider`：回环 OpenAI Responses 上游，按提示词登记回复，可将请求挂起到用例释放；未登记的请求直接失败。
- `velune_acceptance.app.Driver`：打开应用、等待快照条件、重启并重新打开会话，失败时尽量取消并关闭。

用例只用合成数据和回环上游。需要真实账户或真实会话的验收不进入本套件，按[隔离验证与敏感数据](../../docs/development.md#隔离验证与敏感数据)另行授权执行。
