# 验收套件

可重放的验收用例：通过生成的 Python 绑定驱动实际 application，在私有 HOME 与 `VELUNE_HOME` 中复现用户路径。方针与分层见[验证方式](../../docs/development/validation.md#验收套件)。

## 运行

入口需要 Python 3.11 或更新版本。

```sh
# 构建宿主机绑定库并生成匹配的 Python 模块，然后运行全部用例
python3 tests/acceptance/run.py --build
# 复用已生成的绑定，只运行一个用例，并传入实际 Codex CLI
python3 tests/acceptance/run.py codex_conversation_lifecycle --codex /absolute/path/to/codex
# 用例按 REQUIRES 声明所需 CLI（--codex、--claude、--pi、--node），未提供的跳过
```

同一入口也注册为 `svc run acceptance`。缺少外部程序的用例记为跳过而非通过。默认绑定目录为 `target/acceptance/bindings`；`--bindings` 可指向另一次构建生成的目录，用于比较两个版本。`--resources` 指定某次构建自带的运行时资源，缺省时使用源码中的 helper 脚本与 `target/runtime-support` 的依赖。

默认包含不需要 CLI 的配置持久化与历史缓存用例；其它用例按所需 CLI 明确跳过。`configuration_persistence` 经生成的 Python 绑定保存提供商与合成 key，关闭后重新打开核对，再修改配置、保存未发送正文和二进制附件，第二次重开后读取原始字节。它证明跨语言调用与实际持久化路径；不代替真实 Harness 发送或原生界面验收。

每次运行在 `target/verification/acceptance/<run>/` 写入 `manifest.json`，记录当前 runner 工作树的 HEAD、是否有未提交改动、外部程序路径与版本，以及逐个用例的状态、已证明的检查项、耗时和错误。`inputs` 另外记录实际动态库、生成 Python 模块、runner／用例／支持模块和运行时资源的路径与内容摘要；资源目录的摘要包含所链接依赖的内容。`--build` 复用仓库的 `build-support.py` 生成绑定并记录配套回执，加载前核对回执中的库与生成文件摘要，拒绝生成失败后留下的新旧混合文件；manifest 保留回执。使用外部 `--bindings` 时，没有回执会如实记为 `null`；当前工作树的 HEAD 不代表该外部动态库的编译来源。

比较修复前后版本时，使用同一份用例，并给每版传入配套的 `--bindings` 与 `--resources`，核对 `inputs`；不要将旧库配上当前源码 helper 后宣称为旧版结果。验收期间保持这些输入不变。每个用例另有完整日志与结果文件，任务包引用清单与结论。

父进程同时核对 worker 的退出码和结果。用例报告 `passed` 时，必须已通过 `ctx.passed` 记录检查，且各项都成功。worker 即使已写入 `passed`，非零退出也会记为失败。缺失、损坏或不匹配的结果同样生成可读的失败记录。任一失败使 runner 返回非零；跳过不会冒充通过，但没有失败时返回零，因此整体完成范围以清单中实际通过的用例为准。runner 自身的子进程与输入来源回归无需编译，可运行：

```sh
python3 -m unittest discover -s tests/acceptance -p 'test_*.py'
```

## 编写用例

用例是 `cases/` 下的一个模块，声明 `REQUIRES`（所需的外部程序，例如 `("codex",)`）并实现 `run(ctx)`。runner 为每个用例启动独立进程，使用私有临时 HOME、状态与项目目录。遵循代理环境变量的客户端直连回环地址，其他请求被导向本机关闭的端口。需要限制直接出站连接时，由用例启用进程沙箱。

- `ctx.bindings`：本次加载的 `bindings`、`application_api`、`conversation` 模块。
- `ctx.isolated`：`home`、`state`、`project`、`resources` 路径，`dir(name)` 创建附加目录。
- `ctx.tool(name)`：外部程序的绝对路径；未提供时用例被跳过。
- `ctx.passed(check)`：按顺序记录一项已证明的用户可见保证；`ctx.facts` 记录附加观测值。
- `velune_acceptance.responses.ResponsesProvider`：回环 OpenAI Responses 上游，按提示词登记回复，可将请求挂起到用例释放；未登记的请求直接失败。
- `velune_acceptance.app.Driver`：打开应用、等待快照条件、重启并重新打开会话，失败时尽量取消并关闭。

用例只用合成数据和回环上游。需要真实账户或真实会话的验收不进入本套件，按[隔离验证与敏感数据](../../docs/development.md#隔离验证与敏感数据)另行授权执行。
