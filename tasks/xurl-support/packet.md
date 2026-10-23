# xURL 支持

实现与验收已完成：共享核心、MCP、官方 CLI 分发，以及 Mac／Windows／Huawei Android 的深链主路径均已验证。2026-10-23 用户授权“整理一个提交”，本次提交覆盖本任务的源码、构建脚本、验证脚本与文档；此前“未授权提交”等表述为当时的历史状态。

## 当前收敛：点击深链直接打开

用户最新要求“开始精简”，并明确要 deeplink，否定手动复制地址再查找的交互。此决定替代下文历史记录中的复制／粘贴入口：三端删除新加的输入、查找和复制控件，点击系统或会话正文的 `agents://` 链接后自动定位，唯一匹配直接打开，仅多匹配或失败显示结果。正文链接使用来源会话设备；Windows 外部链接使用本机，Mac 使用接收窗口设备，Android 外部链接只有一个已配对设备时自动使用，否则选择设备。保留共享 URI 校验、实例歧义处理、既有运行时执行门禁、官方 CLI 与必要的系统激活。

当前改动只做三端构建与深链主路径验证；停止新增配对回归、镜像操作和组合矩阵。Android 只经 ADB，Windows 只经 `win-ws.localhost` SSH。下文验收记录对应精简前版本，最新证据在末尾补充。

## 目标与决定

2026-10-23 用户要求「引入对 xurl 的支持」，确认对象为 [Xuanwo/xurl](https://github.com/Xuanwo/xurl)。用户选择会话地址互通、Agent 使用 xurl 查询／读取／继续；底层实现可吸纳有利于效率、稳定性和支持范围的部分，不要求替换。原话归 [S47](../../docs/sources.md#s47)。已授权实现，未授权提交。

advisor 复核了接入边界：上游 URI 不携带 Velune 实例或设备身份；公开写入 sink 只提供会话就绪与文本增量，直接启动原生 CLI，不承载 Velune 的审批回复、取消、模型路由或后台工作归属。因此保留现有运行时适配器，官方 CLI 与 Velune MCP 分别承担独立原生执行和应用管理的续聊。SSH 不复制本机架构的二进制。

## 实现

共享核心的 `agent_uri` 生成和定位 Codex、Claude Code、Pi 地址，结果包含所有匹配与逐实例失败。唯一匹配且无失败才允许自动选择；Pi 使用会话头 UUID，保持其内部文件路径身份不变。Codex 精确定位包括原生归档；子会话保持父关系和只读限制。复合会话不伪装成一个原生 URI，涉及多段的 MCP URI 请求要求改用明确 Velune ID。

MCP 新增使用指引、URI 查询与按 Unicode 字符分页的历史读取，revision 防止跨页读到变化后的内容；发送、状态、等待、取消接受 URI 和可选实例 ID。所有执行仍使用既有 owner 与门禁。`list_agents` 在本机可用时返回官方 CLI 绝对路径与目标 HOME、配置根、Harness binary 及 Node PATH，不写入用户的原生技能。

官方 CLI 固定在提交 `24eedb2ff2b141b025efa1a1658fd7f3e88cdd01`，显示版本 `0.0.0-dev`，由 `scripts/install-xurl.sh` 构建。Mac bundle 复制并签名该二进制，保留 Apache-2.0 许可证、源码 revision、第三方声明与 manifest。Mac 增加拷贝、文件菜单粘贴及系统 `agents` scheme；选择结果继续使用现有打开入口。轻量客户端共享 typed command，Android 合成展示夹具已适配新增可选字段。Windows 和 Android 后续接入见下文。

## 验证证据

开发验证仅用临时 HOME、合成历史与合成 Harness，不读取真实会话、凭据或调用真实模型。

- 核心 URI 边界和 Unicode 分页／revision 两个测试通过，日志 `/tmp/velune-xurl-tests.log`、`/tmp/velune-xurl-page-tests.log`。
- `cargo clippy --locked -p velune-bindings --all-targets -- -D warnings` 通过，日志 `/tmp/velune-xurl-clippy-final.log`。
- `cargo check --locked -p velune-bindings --no-default-features` 通过，日志 `/tmp/velune-xurl-portable.log`。
- `manual-xurl.py` 通过多实例、未命中、部分失败、Pi 重复 UUID、原生子会话、官方 CLI 读取三类历史和合成原生续聊，以及 MCP 查询／分页／发送／等待／取消和忙碌／审批／子会话拒绝，最终使用随包 Resources 与签名后 CLI 复验通过，日志 `/tmp/velune-xurl-bundle-test.log`；Node PATH 交付也已核对。
- 原有 `manual-collaboration.py` 回归通过，日志 `/tmp/velune-xurl-collaboration.log`。
- Shell 语法、Python 编译、Markdown 相对链接与标题锚点、`git diff --check` 通过。

Mac 最终构建与签名检查通过，产物为 `target/macos/Velune.app`，日志 `/tmp/velune-xurl-build-final.log`。构建命令为 `CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo bash scripts/build-macos.sh --build-only`，未安装或替换用户现有应用。最初 Cargo registry 符号链接导致 Askama 相对路径无效，改用缓存真实目录重建，未修改依赖源码或版本。

已用独立 bundle ID 与 `/tmp/velune-xurl-gui` 合成配置检查 Mac 实际界面：文件菜单打开地址输入、重复地址列出 claude-a／claude-b、选择 claude-b 后载入其合成历史、唯一地址直接载入 xURL unique、会话右键菜单出现拷贝 xURL 地址。原外置卷隔离启动曾在文件访问处等待；将验收包与合成来源放入 `/tmp` 后完成，没有调整系统权限。系统 URL scheme 的注册与路由代码已核对，未单独验证系统外部点击链接。验收应用已关闭。

## 已知范围

应用 URI 查询只支持标题 q 与 limit，官方 CLI 处理更广的路径、角色和 Pi 条目。官方 `-d` 使用独立原生认证和执行，不继承 Velune 审批、模型路由或取消；指引优先推荐 MCP 继续管理中的会话。SSH 使用既有 MCP，远端官方 CLI 需要用户自行安装；Pi 自定义 sessionDir 不宣称官方 CLI 能读取。真实账户、外部 SSH 运行时及公证未验收；Windows／Android 的新增证据与剩余范围见下文。

## Windows 与 Android 后续接入（2026-10-23）

用户随后要求「给 windows 也接入，安卓也可以试一下」，授权两端实现与必要设备验证。Windows 补随包官方 CLI、拷贝／粘贴／系统链接与同 HOME 的激活重定向；通知激活保留原有入口。Android 仍是纯控制端，设备内粘贴使用当前设备，外部 URI 先选择已配对设备，再由该设备核心解析；不打包桌面 CLI。两端均复用匹配和 failures，不在 UI 拼接 URI 或推测原生身份。advisor 已只读复核上述边界。

当前主 Agent 负责两端实现、构建和合成验证；没有源码子任务。Windows 的 SSH 构建主机可达，Android 当前有两台 USB 设备。验证只用独立合成配置，保留此前未提交的 Mac 与核心改动；仍未获准提交 Git。


### 实现与验证收敛

Windows 已补齐随包 `xurl.exe`、许可证与 REVISION、复制和粘贴入口、系统协议注册、同 HOME 激活重定向；继续使用现有 `ShowDialog`、Problems、Core 与打开会话入口。通知启动交由同一个激活入口处理，避免冷启动重复路由。通过 `win-ws.localhost` SSH 在独立源码目录 `C:\Users\yyh\velune-xurl-support` 构建，未改远端原源码目录。原有 runtime-support 和生成工具用于复用构建依赖。

Windows 的官方 CLI 读取合成 Claude 会话、URI 唯一匹配打开、多实例选择并打开 claude-b、已有进程接收协议激活均通过。浅色和深色界面检查通过，正常退出码为 0，无 crash.log。冷启动协议测试发现 XamlRoot 尚未就绪，现仅在 xURL 入口等待 Root.Loaded 后再显示对话框；最终在未运行状态以协议参数启动后，已观察到地址对话框与 claude-a／claude-b 两项匹配，未再出现 XamlRoot 错误。复验后正常退出，退出码 0，无 crash.log；临时计划任务已删除，系统主题已恢复。构建日志 `/tmp/velune-xurl-windows-final-build.log`，界面与激活证据位于 Windows 合成目录 `C:\Users\yyh\velune-xurl-ui`。

Android 增加链接入口、当前设备内输入、外部链接选择设备、会话长按复制与共享查询结果展示；不加入执行核心或 CLI。remote-only ARM64 构建、正式 APK 编译与 lint 通过，日志 `/tmp/velune-xurl-android-native.log`、`/tmp/velune-xurl-android.log`。Android 9 和 16 的显式 VIEW intent 可启动测试 Activity；Android 16 已观察到地址输入、未选设备时禁用查询，以及热启动 intent 送达。远端查询到打开会话的完整链路未验收，不能将共享核心测试当作真机端到端证据。

用户指出镜像／computer use 不合适，要求 Windows 只经 SSH、Android 用 ADB；后续遵守。早期曾安装 scrcpy 并创建两个临时镜像 bundle，均已关闭、删除，scrcpy 已卸载。首轮 Android 隔离 applicationId 配置未生效，更新了两台连接手机的原 debug 包；修正后使用 `app.velune.android.xurltest`，该独立包已从两台手机卸载。没有读取真实会话；没有完成新的设备配对。临时合成执行端已关闭。

用户要求立即检查过度设计与 ROI。只读复核认为共享 URI 契约、实例歧义处理、必要平台入口与 Windows 原生激活值得保留，没有为假设场景增加适配器框架；过度主要来自验证工具搭建、双手机与完整配对回归。停止这些验证，不新增组合矩阵或平台测试框架。只修复并复验已复现的 Windows 冷启动主路径问题，随后完成构建记录、文档与差异检查。未获准 Git 提交。

## 深链精简后的验证（2026-10-23）

已删除三端手动地址输入、查找按钮和复制 xURL 菜单，删除不再使用的 Mac 地址展示字段与 Android 地址表单展示夹具。保留系统链接接收，补正文 Markdown 链接的设备上下文路由；Windows 不再把 agents URI 当作文件路径，Android 收到地址后自动解析。MCP 使用指引明确输出可点击的 Markdown 会话链接。

Windows 通过 SSH 在合成 HOME 冷启动协议地址，直接出现 `xURL unique` 会话，无地址框或查找按钮；随后点击合成正文的 agents 链接，出现 claude-a／claude-b 选择列表，选择 claude-b 后打开该实例的合成会话。对 UI Automation 树做断言，证据为 `/tmp/velune-xurl-deeplink-windows-tree.txt`、`/tmp/velune-xurl-deeplink-windows-choice.txt`、`/tmp/velune-xurl-deeplink-windows-opened.txt`。正常退出码 0，无 crash.log；临时计划任务与 UI runner 已停止。未重复主题矩阵。

Android 编译与 lint 通过，日志 `/tmp/velune-xurl-deeplink-android.log`。另外构建独立 applicationId 的测试 APK，并用 aapt 确认包名 `app.velune.android.xurltest`；ADB 安装到 Redmi 时返回 `INSTALL_FAILED_USER_RESTRICTED: Install canceled by user`，本轮真机验证至此停止，未修改安装限制、未重试、未新配对。深链直接打开远端会话仍未真机验收，不能把编译通过当作端到端证据。

Markdown 相对链接与锚点检查通过（287 项），`git diff --check` 通过。未提交 Git。

最终 Mac 构建与签名通过，日志 `/tmp/velune-xurl-deeplink-mac-final.log`；Windows 通过 SSH 重新构建通过，日志 `/tmp/velune-xurl-deeplink-windows-final.log`，两者均包含更新后的 MCP 链接展示指引。本轮未重跑 Mac 实际点击链路，保留构建通过与运行验收的区别。

## Huawei 真机深链验收（2026-10-23）

用户明确指定 Android 使用 Huawei，不使用 Redmi Turbo 3。后续本任务的 Android 操作固定目标 `HYJ4C19524006587`（JKM-AL00a，Android 9），只经 ADB。

已确认独立 APK 的 applicationId 为 `app.velune.android.xurltest`，并成功安装到 Huawei；原应用未覆盖。复用现有合成 Claude 历史，为隔离包预置一个经核心正常批准的合成控制端身份与连接，不测试添加设备流程。首次预置文件默认权限不符合身份文件安全要求，核心拒绝启动；将测试身份及状态文件设为 0600 后正常工作，未为测试准备问题修改产品代码。

真机上已验证：冷启动 VIEW 深链自动进入 `xURL unique`，无需设备选择、地址输入或查找；点击正文中的 agents 链接出现 claude-a／claude-b，选择 claude-b 后进入 `xURL fixture` 并显示合成历史；应用已运行时再次接收外部深链，直接回到 `xURL unique`。对 ADB UI hierarchy 做了标题、正文和无手动查找控件的断言。证据为 `/tmp/velune-xurl-huawei-cold.xml`、`/tmp/velune-xurl-huawei-choice.xml`、`/tmp/velune-xurl-huawei-linked.xml`、`/tmp/velune-xurl-huawei-warm.xml`。测试使用同局域网的合成执行端，不读取真实历史、不发送真实模型请求。

验收后独立测试包已卸载、手机临时 XML 已删除、合成执行端已关闭。本轮未操作 Redmi。未改产品源码、未提交 Git。

## 本机 Mac 深链验收（2026-10-23）

用户要求本机器也验证。复用最终构建，复制为独立 bundle ID `local.velune.xurl-validation`，通过 Launch Services 启动 `/tmp/VeluneXurlValidation.app`，显式设置 `VELUNE_HOME=/tmp/velune-xurl-gui/velune`，只访问既有合成会话。

冷启动时随 `open -n -a` 传入唯一 agents URI，实际窗口直接显示 `xURL unique`、claude-a 与合成正文，没有地址输入或查找表单。点击正文的原生链接后出现 claude-a／claude-b 选择列表；选 claude-b 并打开，窗口显示 `xURL fixture`、claude-b 和合成历史。应用运行中再次由 `open -a` 传入唯一 URI，当前窗口切回 `xURL unique` 和 claude-a。三项均通过原生辅助功能树观察确认。

验证后正常退出隔离应用并确认进程消失，已注销临时 bundle 的 Launch Services 注册。原 `/Applications/Velune.app` 未替换，未调用真实模型；本轮未改产品源码或提交 Git。
