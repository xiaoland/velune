# 开发与复验

以类型安全、静态检查和构建为先。行为验收使用手动操作或临时端到端脚本，不建立成套的单元或集成测试，也不把临时脚本接入 CI。自动化测试只为类型与静态检查无法表达的真实不变量少量保留，以 `cargo test -p <包>` 在相关平台上运行。

内部不变量优先由类型表达：解析后的对象携带所需关联，枚举区分有效状态，穷尽匹配让新增变体触发编译错误。已在配置入口验证的不可变数据不在每次派发时全量重验。lint 用于有明确代码模式和实际维护收益的规则，不以扫描字符串或禁止所有条件分支代替类型设计。文件内容、用户配置、HTTP 与子进程响应仍在接入边界解析；运行时版本、目标存在性和实际执行失败不能预先由编译器证明。

## 隔离验证与敏感数据

开发验证默认使用临时 `VELUNE_HOME`、合成凭据、会话与项目。除当前任务明确授权的操作外，不读取、复制、输出或提交真实登录状态、令牌、API key、支付数据或 Harness 会话记录，也不调用真实计费服务或公网联调。获准进行真实联调时，只操作授权范围内的账户、服务与请求；版本、范围和结果记录在对应 Task Packet。合成验证通过不代表真实账户认证、供应商规则或完整产品体验已通过。

## 验证方式

Rust 工作区的静态检查：

```sh
cargo fmt --all --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo clippy --locked -p velune-bindings --no-default-features --all-targets -- -D warnings
cargo clippy --locked -p velune-bindings --no-default-features --features remote --all-targets -- -D warnings
cargo tree --locked -p velune-bindings --no-default-features --features remote
```

`velune-bindings --no-default-features --features remote` 是纯控制端，依赖树不含 application、agent-runtime、gateway 或 ai-provider；完全禁用 feature 时没有本地或远程构造入口。application 自身禁用 `local-runtime` 仅裁掉 Harness，配置校验仍链接 gateway，不是纯远端客户端的装配路径。

根目录 [`clippy.toml`](../clippy.toml) 禁止直接调用 `Command::new`。子进程经 `velune_agent_runtime::process::command` 创建（ai-provider 不依赖 agent-runtime，使用自己的 `hidden_command`），长期运行的进程树经 agent-runtime 的 `process_tree` 创建：Unix 使用独立进程组，Windows 使用 kill-on-close 的 Job Object 并加 `CREATE_NO_WINDOW`，避免 GUI 宿主为每个子进程弹出控制台。

平台代码的检查是各平台构建本身（Swift、Kotlin、C# 均把警告视为错误）。平台中可脱离界面的逻辑放在独立文件，以脚本单独编译检查：`bash scripts/check-token-units.sh` 用同一组样例检查 Mac 与 Windows 的 token 数量格式化与解析；`bash scripts/check-account-quota.sh` 在 `build-macos.sh --build-only` 之后，以实际 Swift 绑定检查 Mac 的额度呈现。

端到端行为用临时脚本验收：先构建动态库，并按 [bindings unit](../packages/bindings/README.md#生成绑定) 生成匹配的 Python 绑定，再在临时 `VELUNE_HOME` 与临时 Harness HOME 中，以合成凭据和本机回环上游驱动实际 application。需要实际 Harness 时传入已安装 CLI 的绝对路径。脚本随任务使用，不提交到仓库；所用版本、覆盖范围与结果记录在该任务的 Task Packet。界面检查使用隔离的应用实例或下文的预览模式。

## 开发环境

在当前 Mac 开发工作区使用 `svc dev` 管理已声明的长期服务。配置归仓库 [svc.json](../svc.json)，本机已验证 SVC CLI 16.0.2（schema 4）。CLI 仅用于开发；产品运行不依赖它。`svc dev status` 只观察，`ensure` 复用或启动一个目标，`stop` 运行该目标声明的停止动作。

| 目标 | 能力与启动依赖 |
| --- | --- |
| `lgtm` | sfp7 上已有的 Docker Compose 接收端，停止保留数据卷 |
| `telemetry` | sfp7→本机的 Grafana 30300 与 OTLP/gRPC 34317；先确保 `lgtm` 就绪 |
| `telemetry-windows` | 本机→Windows 的相同回环端口；先确保 `telemetry` 就绪 |
| `telemetry-android` | 唯一 USB Android 的 ADB reverse 34317；先确保 `telemetry` 就绪 |

```sh
svc dev status
svc dev ensure telemetry
svc dev ensure telemetry-windows
svc dev ensure telemetry-android
svc dev stop telemetry-android
svc dev stop telemetry-windows
```

SSH 和 ADB 继续承担实际传输；[服务动作](../scripts/dev_services.py)只把就绪和有界停止交给 SVC，不维护另一套进程注册表。SSH control socket 位于本机 `~/.local/state/velune/dev/`，不会因工作树切换重复开通道。停止一个通道不级联停止上游。上游恢复时复用仍存活的下游隧道，端口冲突不接管其他服务。SVC 回执提供实际操作日志位置；也可用 `--json` 获取机器结果。

共享配置使用已有 SSH 别名 `sfp7-ws.localhost`、`win-ws.localhost`，Android 默认选择唯一 USB 设备。需要不同主机或指定设备时，在被 Git 忽略的 `svc.local.json` 中用 schema 4 稀疏覆盖相应目标的 `probe.argv`、`provision.argv` 和 `stop.argv`；设备参数为 `--device <serial>`，ADB 路径可用 `--adb` 指定。切换物理目标前先停止旧目标，并修改 `host_key`，避免与另一设备共享协调身份。认证仍由 SSH 和远端私有文件管理，不写入这些声明。

## Mac 构建

Mac 应用的结构与界面见 [Mac README](../app/mac/README.md)。先安装历史读取依赖与 xURL，再构建并安装：

```sh
./scripts/install-runtime-support.sh
bash scripts/install-xurl.sh
bash scripts/build-macos.sh
open /Applications/Velune.app
```

构建脚本先生成 UniFFI 模块，再用 SwiftPM 的 Xcode 后端构建本机架构产品，固定 macOS 15 部署底线。swift-markdown 0.9.0 的 manifest 需要 Swift 6.2 或以上工具链。应用与生成绑定目标各自启用 warnings-as-errors，依赖保持其声明的 Swift 语言模式；不构建或执行依赖的测试 targets。生成文件只放在 `target/` 下。

RelayKit 接入还需要 Go 工具链，桥接模块版本及依赖由 `packages/ai-provider/relaykit-bridge/go.mod` 与 `go.sum` 固定，Go 最低版本以该模块声明为准。Go 转换库作为同进程依赖链接，不启动独立网关后台。bundle 的 `ThirdParty/RelayKit` 保留上游、Go runtime 和传递依赖许可证以及模块锁文件；构建 manifest 记录 Go 版本、两个模块文件的摘要、源码提交、UniFFI 版本、类型接口和配置 schema。

构建脚本默认将验证签名后的 bundle 安装到 `/Applications/Velune.app`；仅构建用 `bash scripts/build-macos.sh --build-only`。安装器不覆盖尚未退出的应用：先正常退出 Velune，应用拒绝退出时可终止已确认的 Velune 进程后安装。产品显示版本来自仓库 `VERSION`，“关于 Velune”面板读取同一版本。

签名默认为 ad-hoc。macOS 隐私权限（如“可移除宗卷上的文件”）按签名记住，ad-hoc 签名每次构建都不同，系统因此在每次重装后重新询问，回答之前对应位置的读取一直阻塞。设置 `VELUNE_CODESIGN_IDENTITY` 为钥匙串中的代码签名身份（如 Apple Development 证书，`security find-identity -v -p codesigning` 列出名称或 SHA-1）则以它签名，授权跨构建保留。这只是本机签名，不是公证或分发签名。

若 UniFFI 模板宏报告 `askama.toml` 不存在而该文件实际存在，原因是 Cargo registry 经符号链接放到了其他卷，Askama 计算出无效的模板相对路径；把构建命令的 `CARGO_HOME` 指向缓存的真实目录即可，不需要修改 registry 或第三方源码。xURL 的构建同样适用。

应用根目录由启动进程的 `VELUNE_HOME` 指定，未设置或留空时为 `~/.velune`，非空值必须是绝对目录路径；平台将该路径显式传给 Application，库不自行读取环境。Finder 启动通常不继承 shell 配置，隔离开发直接带环境运行 bundle 内可执行文件：

```sh
VELUNE_HOME=/absolute/path/to/isolated-home /Applications/Velune.app/Contents/MacOS/Velune
```

隔离视觉预览使用 `--preview`（合成多轮会话）或 `--preview-empty`。预览 Store 无 Transport，不打开 Application、不访问真实配置、Keychain 或会话、不调用模型，因此不能证明真实循环。

共享品牌原稿归仓库级 [assets/brand](../assets/brand)。Mac 应用图标由 `scripts/render-app-icon.swift` 从该目录的 graphite 光学稿生成 ICNS；Dock 图标保留 macOS 的透明光学留白，绘制范围在生成器中维护，不通过修改品牌 SVG 补偿。

## iPad 与 iPhone 构建

运行 `scripts/build-ios.sh` 构建 arm64 模拟器，使用 `scripts/build-ios.sh iphoneos DEVELOPMENT_TEAM=<团队ID>` 构建设备版本。单一 Xcode target 支持 iPad 分栏与 iPhone 紧凑导航，只链接 remote-only 核心。工具链、签名、合成预览与边界见 [iOS README](../app/ios/README.md)。

## Android 构建

`scripts/build-android-native.sh` 构建 API 28+／arm64-v8a 的 remote-only Rust so、匹配的 Kotlin 绑定与固定摘要的 rustls verifier AAR，产物在 `target/android-native` 与 `target/android-bindings`。`scripts/build-android.sh` 先执行原生构建，再构建 debug APK 并运行 Android lint；Kotlin 警告视为错误。只改 Kotlin 时可加 `--skip-native`，此时绑定必须与已有 so 配套。安装与运行见 [Android README](../app/android/README.md)。

控制端的隔离验收用合成的完整端：Python 绑定打开临时 application、合成 ACP 运行时与公共 iroh relay，生成邀请并在核对后批准，手机侧使用真实 App。用 adb 驱动时可以临时把邀请写入应用私有目录，这类 hook 不入库。

## Windows 构建

Windows 使用 MSVC 目标（`x86_64-pc-windows-msvc`，工具链版本同 `rust-toolchain.toml`），需要 Visual Studio Build Tools、.NET 10 SDK、Go、Python 3（收集随应用分发的 Rust 许可全文）和 [llvm-mingw](https://github.com/mstorsjo/llvm-mingw)。RelayKit 桥依赖 cgo；MinGW 静态库无法可靠链接进 MSVC 产物，所以 Windows 把它构建为 `velune_relaykit.dll`，Rust 以 `raw-dylib` 绑定，无需导入库。构建脚本把 DLL 复制到 `target/<profile>/`，应用分发时须放在可执行文件旁。cgo 编译器默认为 `<arch>-w64-mingw32-clang`，可由 `VELUNE_RELAYKIT_CC` 指定绝对路径；PATH 没有传递给 build script 时，同时用 `GO` 指定 Go 的绝对路径。

先安装 runtime-support，并执行 `powershell -ExecutionPolicy Bypass -File scripts\install-xurl.ps1` 准备官方 xURL CLI。`scripts/build-windows.ps1` 先构建 `velune_bindings.dll`，再由固定提交的 `uniffi-bindgen-cs` 从中生成 C# 绑定，最后 `dotnet publish` 出自包含的 `Velune.exe`。界面、运行与已知限制见 [Windows README](../app/windows/README.md)。

开发用 Windows 机（SSH 别名 `win-ws.localhost`）上只有一个仓库 `F:\CODING\Velune`：main 的普通克隆，origin 指向 GitHub。未推送的提交以 bundle 送达：Mac 上 `git bundle create <文件> <对方 main>..main`，复制过去后在该仓库 `git pull --ff-only <文件> main`，再删除 bundle。runtime-support、xURL 与 `uniffi-bindgen-cs` 在被忽略的 `target\` 中，新克隆或 worktree 需从已有的 `target\runtime-support`、`target\tools` 复制。并行或临时的验证副本用 `git worktree add F:\CODING\.worktrees\<任务>\velune`，用完即 `git worktree remove`。日常使用的版本先退出正在运行的 Velune，再 `robocopy target\windows\Velune %LOCALAPPDATA%\Programs\Velune /MIR` 安装，桌面快捷方式指向那里，因此重新构建不受运行中的应用影响。

## 原生运行时与历史读取

Velune 不分发 Agent runtime、Pi SDK、Codex CLI 或 Node。各 Harness 由用户安装，运行时实例保存可执行文件的绝对路径，不依赖 Finder 的 PATH；版本类型与兼容区间见 [agent-runtime](../packages/agent-runtime/README.md#版本与原生控制)。

| 运行时 | 执行机器上需要 |
| --- | --- |
| Pi 1.0 | Pi CLI 与 Node 22.19+ 的绝对路径 |
| Codex 0.159 | Codex CLI 与 Node 22.19+ 的绝对路径（历史直接调用 app-server） |
| DeepSeek Harness 0.2 | DSH 包的 JavaScript 入口与 Node 22.19+ 的绝对路径 |
| Claude Code 2.1 | Claude CLI；历史读取使用 Python 3 执行 Rust 内嵌的只读 helper |
| 通用 ACP | ACP 程序或 bridge 的绝对路径；协作 MCP 的 stdio 回退需要 Python 3 |

[`install-runtime-support.sh`](../scripts/install-runtime-support.sh) 按 [`package-lock.json`](../packages/agent-runtime/runtime-support/package-lock.json) 把 huihua 及其依赖安装到忽略的 `target/runtime-support`，禁用 npm lifecycle scripts，不修改全局安装。Mac 构建只分发其 parser 依赖和薄 adapter helpers，并带完整许可声明与依赖锁，清单见[第三方声明](../packages/agent-runtime/runtime-support/THIRD_PARTY_NOTICES.md)。`install-pi-runtime.sh` 仅供开发者在忽略的 `target/pi-runtime` 安装一份隔离的外部 Pi，用作验收时的 `--pi` 路径，不是构建前置条件，也不进入应用包。

SSH 运行时依赖本机 OpenSSH，以及远端 POSIX 环境、Python 3 和匹配版本的 Harness；采用非交互认证与严格主机校验，应由用户预先完成认证和主机信任。Windows 宿主暂不支持 SSH 运行时（依赖 OpenSSH ControlMaster 与 Unix socket）。执行位置与 HOME 的契约见 [agent-runtime](../packages/agent-runtime/README.md#原生提供商远端与-home)。

## 本地配对 CLI

启动 Velune 后，在仓库运行 `scripts/velune` 操作该应用正在使用的远控服务；CLI 不另开设备身份或执行核心。Mac 默认连接 `$VELUNE_HOME/remote/cli/control.sock`（未设置时为 `~/.velune`）。Android debug 应用通过 `--android <adb序列号>` 和 `run-as` 访问同一接口，设备的 toybox 需支持 `nc -U`；release 包不启动此入口。监听仅使用私有目录中的 Unix socket，不开放 TCP 端口。

```sh
scripts/velune status
scripts/velune invite | scripts/velune --android <adb序列号> begin
scripts/velune --android <adb序列号> status
scripts/velune approve --attempt <本次attempt> --peer <控制端peer> --code <两端一致的八位核对码>
scripts/velune --android <adb序列号> connect --peer <被控端peer>
scripts/velune revoke --peer <控制端peer>
```

`invite` 与界面的“添加控制设备”相同，会开启控制入口并生成五分钟邀请；输出含邀请秘密，应直接传给 `begin`，不要保存到日志。其余命令返回 JSON，失败退出码非零。`status` 只返回远控状态，不读取会话正文。批准要求当前 attempt、预期 peer 和核对码同时匹配；CLI 与界面共用授权规则。应用退出时接口关闭；修改 `VELUNE_HOME` 或使用隔离实例时，CLI 必须连接对应实例的 socket。

## 本地诊断

Mac 使用 `OSLog.Logger`，subsystem 为 `local.velune`，系统 Console 可按它筛选。Rust 的 `tracing` subscriber 由客户端遥测 owner 装配，JSON Lines 日志位于 `VELUNE_HOME/logs/velune.<日期>.jsonl`，按 UTC 日期轮转并保留最近 7 个文件；只限制文件数量，没有总字节上限。日志目录在 Unix 上为 0700 权限。领域包只发事件，不创建 subscriber；跨线程显式传播 dispatcher 和 span。

失败信息带诊断编号；用它查找日志中的 `operation_id`，可定位操作、错误代码、阶段、耗时与底层原因。Swift 的本地操作编号另行命名，不是 Rust trace ID。常规 snapshot 和认证轮询不产生成功日志。诊断保存实际错误链、helper stderr 与失败上下文，默认不采集请求、配置或会话正文；显式启用的 OTLP 只导出批准的操作元数据。

网关请求另有 `gateway_request` span 的 `request_id`，与建立执行准备时的 `operation_id` 不同；入口、路由、单次派发与结束事件的含义见 [AI 服务](design/ai-service.md)。运行时 stderr 持续排空并在有界缓存中保留用于失败诊断，超出时明确标记截断。

## 开发遥测

普通启动与 Debug 构建默认不启用遥测。使用 `scripts/dev-telemetry.py launch` 显式启动隔离实例时，工具通过上述 SVC 入口准备通道并检查 Grafana 与 OTLP 就绪，生成私有临时 profile 和 run manifest。它不修改日常偏好或全局 `OTEL_*`，也不重建／清空 LGTM。业务 home 与 OS home 使用本次运行的独立临时目录，避免长仓库路径超过 Mac 的 Unix socket 长度限制。接收端不可用时继续启动业务并报告未接入；端口冲突不停止无关进程。

先按各端构建说明生成产物，然后在仓库运行以下入口。桌面 `--app` 必须是已安装或已构建产物的绝对路径；Mac 指向 bundle 内可执行文件，Windows 指向 `Velune.exe`。移动端省略 `--app` 直接使用已安装应用；只有明确传入 APK／bundle 才执行覆盖安装。设备必须已启动；入口会重新启动指定应用，以便读取本次临时配置。

```sh
python3 scripts/dev-telemetry.py launch macos --app /absolute/path/Velune.app/Contents/MacOS/Velune
python3 scripts/dev-telemetry.py launch windows --app C:/absolute/path/Velune.exe
python3 scripts/dev-telemetry.py launch ios-simulator --device <Simulator UUID> --package app.velune.ios
python3 scripts/dev-telemetry.py launch android --device <ADB serial> --package app.velune.android.telemetry
```

Android 复用已安装的 debug 应用和合成配置。工具经 ADB reverse 把设备回环 34317 转发到开发机，只通过 `run-as` 的 stdin 写入 debug 应用私有 profile，不经过共享存储。该转发需要保留 ADB；手机与桌面的回环地址没有自动互换。Simulator 使用容器私有文件和 `SIMCTL_CHILD_*`，真机需要另行验证可达的受控入口。Windows 的原生 OpenSSH 不支持多路复用，工具复用显式准备的回环 30300／34317 转发并检查就绪；当前开发环境由 `svc dev ensure telemetry-windows` 从 Mac 反向映射到 Windows，不开放 LGTM 到 LAN。

输出的 `manifestPath` 指向 `target/telemetry-development/<run-id>/run.json`。它含 run ID、应用路径、工作区当前提交（`workspaceRevision`）、进程／设备、状态路径、查询凭据文件位置与保留范围，不含认证值。该提交只标识启动工具所在工作区，不代表已安装应用的构建版本。复制的构建目录可以没有 Git，此字段为 null，不阻止应用启动。读取状态与限定本次运行的查询：

```sh
python3 scripts/dev-telemetry.py status /absolute/path/run.json
python3 scripts/dev-telemetry.py query /absolute/path/run.json traces
python3 scripts/dev-telemetry.py query /absolute/path/run.json logs
python3 scripts/dev-telemetry.py query /absolute/path/run.json metrics
```

状态 45 秒未刷新或 run ID 不匹配时视为过期；停止进程后不要把旧文件当作当前生效状态。状态文件缺失、容器路径不可读或内容无效时返回 `state_unavailable`，不能解释为关闭。SDK 的 `rpc_completed` 不证明三信号已经存入后台，必须分别查询并检查时间范围、实例、采样和结果限制。精确实时丢弃计数没有统一公开接口，未知值不能视为零；已知 SDK 丢弃汇总留在同目录 `sdk-diagnostics.json`，包含原修订并最多保留 64 条。Swift SDK 没有日志丢弃 hook。正常退出与移动端强制结束的证据边界不同。

查询默认选择 `velune.telemetry.schema.version="2"`。历史数据使用 `python3 scripts/dev-telemetry.py query <manifest> traces --schema-version legacy`，混合版本用 `--schema-version all`；logs 和 metrics 支持同一选项。旧属性 operation／operation_id／outcome 与 v2 的 velune.operation／velune.operation.id／velune.outcome 分开解释，不能把新查询无结果理解为旧记录消失。trace 可能跨版本，查看详细记录时按各自 resource 判断。legacy trace 的缺属性条件使用 Tempo 的 `= nil`，接收端需支持该语法。

语义与扩展范例归[遥测契约](design/telemetry.md#扩展埋点与读取范例)。先限定 run、实例、时间和版本，再沿 trace、operation ID、RequestId 检查受理与执行。Loki 中点号属性会按其映射显示为下划线名称；查询应使用接收端实际字段，不解析日志 body。指标按操作层分别统计，不能将 UI、bindings 与 RPC 次数相加。Gateway 本地诊断的 transport_completed 仍表示传输结束，OTLP 的 outcome 则同时考虑 HTTP 结果；返回完整 503 在 OTLP 中是 failure。
