# Windows MyGO 应用

2026-10-21 起 Windows 端改用 WinUI 3（[S45](../../docs/sources.md#s45)，[Windows WinUI 任务](../windows-winui/packet.md)）。本版只保留到 WinUI 版功能相同，期间只修阻塞问题。

## 当前任务

2026-10-20 用户改定 Windows 端使用 MyGO 而非 GPUI（[S43](../../docs/sources.md#s43)）。目标与 GPUI 版相同：完整端，本机运行 agents，并可控制其他已配对设备；先交付会话日常闭环（会话列表与转录、发送、停止、审批／问题），再补设备配对与设置。Go 桥坚持 purego、不用 cgo。产品约束见 [PRD](../../docs/prd/index.md)，跨单元方案见 [架构](../../docs/design/architecture.md)，远控契约见 [远程访问设计](../../docs/design/remote-access.md)。

GPUI 版已按 hard-cutoff 删除，MyGO 版移入 `app/windows`，构建脚本为 `scripts/build-windows.ps1`（删除时机为开发方判断，用户未单独确认；旧代码在版本历史中）。

## 已核实事实（2026-10-20）

- MyGO（[egoist/mygo](https://github.com/egoist/mygo)，MIT）最新发布 v0.5.5（2026-10-16），两天内连发五个版本，main 仍在更新。原生 UI 为纯 Go 自绘，组件包括 sidebar、split、list、markdown、text input、dialog 等（`docs/ui/`）。`docs/bindings.md` 只涉及 web 前端调用 Go，与 Rust 桥无关。
- 10-19 spike（同为 v0.5.5）的遗留问题：侧栏与输入区未显示、FollowEnd 未停在末尾、工作集约 295 MB（GPUI 约 64 MB）。当时判断可能是样例用法问题，未深究。
- bindings 的 FFI 面（Mac 完整 feature 生成的头文件）：约 155 个函数，即 2 个自由函数、2 个构造器、5 个 object 的 141 个方法，加上 clone／free。没有 async、callback interface，也没有 f32／f64 参数或返回值；观察者用阻塞的 `next(timeout)`。record、enum、error 与集合经 RustBuffer 序列化。

## 方案

1. **Go 生成器**：`velune-gobindgen <library> <out-dir>`（bindings 的 `cli` feature）读取库中的 UniFFI 元数据，输出 `velune/bindings` Go module 到 `target/bindings/go`。生成代码不入库，与 Swift／Kotlin 相同。（已完成，见下）
2. **Go 运行时**：RustBuffer、RustCallStatus、对象句柄与调用助手。（已完成，见下）
3. **冒烟**：在 win-ws 上用 Go 程序打开隔离 `VELUNE_HOME`，列出运行时与会话，并用 ACP fixture 发送一轮。
4. **MyGO 应用**：会话列表与转录、composer、停止、审批，然后是设备切换、配对（二维码、核对码批准）与设置。界面遵循 Windows 交互习惯与项目 UI 规则（双击就地编辑、消息左右布局、不显示头像与昵称）。
5. 达到会话日常闭环后删除 GPUI 版，更新 README 与架构。（已完成）

## 进展（2026-10-20）

- purego v0.11.2 的 `RegisterFunc` 已按各平台 C 约定处理按值 struct 参数与返回（Windows x64：1／2／4／8 字节以外的 struct 经隐藏指针返回、按引用传参；源码 `struct_amd64.go`）。因此生成代码直接声明带 `rustBuffer` 的 Go 函数类型，不手写调用约定；同一包也能在 macOS 加载 dylib。Windows 用 `syscall.LoadDLL` 取地址（purego 在 Windows 无 Dlopen）。
- 生成器遇到 async、callback interface、trait interface、带方法的 record／enum、flat error 时直接报错，不静默丢弃。类型映射：record→struct，无字段 enum→`int32` 常量，带字段 enum→interface＋变体 struct（error 额外实现 `error`，消息取 `Detail` 字段），Option→指针（object 与带字段 enum 用 nil），object→带句柄的指针并提供 `Destroy`，每次调用克隆句柄。
- Mac 冒烟（临时 home、dylib 复制到内部磁盘）：`Load` 校验 contract 与全部 checksum 通过；`ProviderTemplates`、`DecodeRemoteOffer` 的类型化错误、`VeluneApplicationOpen` 与 `List` 快照正常。
- win-ws 冒烟（MSVC release DLL 约 4 分钟；Go 程序在 Mac 交叉编译，`CGO_ENABLED=0`）：同上各项一致；另用最小合成 ACP agent（Python）跑通一轮完整会话：发送、`snapshot_update` 增量合并、权限请求选择允许、运行中取消、`List`、`Shutdown`，结束后无残留子进程。Mac 结果相同。`velune_relaykit.dll` 须与 exe 同目录，Windows 按 exe 目录解析 DLL 依赖。

- 会话日常闭环已写入 `app/windows-mygo`（现为 `app/windows`）（会话列表、设备切换、新会话、转录、审批／提问、发送与停止、就地重命名、已读标记、控制请求对话框），设置尚未迁移。用 MyGO 的 headless Tester 以临时程序（未入库）验证：真实 application＋合成 ACP 会话的渲染在 Mac 与 win-ws 上一致，中文回落字体正常；控制请求对话框起初把焦点给“拒绝”，Enter 即拒绝，现改为焦点落在核对码上，Enter 与 Esc 均不作答。Tester 的公开点击不带系统点击次数，双击重命名无法 headless 验证。
- 设置窗口（设备配对与二维码、运行时发现与手动添加、提供商）已从 GPUI 版迁移，二维码用纯 Go 的 `rsc.io/qr`。构建脚本（当时为 `scripts/build-windows-mygo.ps1`）在 win-ws 上完整构建通过（约 160 秒，`velune.exe` 约 16 MB）。headless 检查（临时 home、合成远控状态，阻塞调用在无窗口时同步执行）在 Mac 与 win-ws 上覆盖：复制与收回邀请、无效邀请的错误、发现运行时（win-ws 实际找到 Pi、Codex、Claude Code，后两者版本过旧无适配器）、手动添加的必填校验与保存、提供商的校验、保存（全角逗号分隔模型）与删除。复制走 `ui.Context.WriteClipboard`，Tester 截获而不碰系统剪贴板。
- win-ws 真机启动（交互会话的计划任务，用户空闲超过 10 分钟才启动，不发送输入，临时 `VELUNE_HOME`）：首次启动即崩溃，因为 MyGO 在 `NewWindow` 内构建首帧，早于保存窗口引用，`SetTitle` 收到空窗口；修复后窗口正常显示（标题、设备切换、设置按钮、空状态），远控服务启动，前台且响应，工作集约 72 MB、私有内存约 150 MB，35 秒内稳定（spike 时约 295 MB）；正常关闭，无残留进程。

## 以 Mac 为基准对齐（2026-10-20）

用户确认 Windows 贴近 MyGO 自身风格、交互结构以 Mac 为基准，Android 推后单独处理（原则沉淀在 [DESIGN.md](../../DESIGN.md)）。

- 主窗口：输入区操作行与 Mac 同构，含“运行时 · 模型 · 推理强度”三子菜单、权限模式、未发送会话的工作目录，下一轮可换运行时。选择经核心 `ComposerChoices` 按“会话 × 运行时”保存；新会话改为空会话。侧栏新增 Lucide 图标按钮，Ctrl+N／Ctrl+, 生效。会话选项、附件、队列、状态栏不在本轮。
- 设置：分类与顺序同 Mac（AI提供商、Agent 运行时、设备）。前两项改为列表，带＋／−、“编辑…”、双击编辑、右键菜单与删除确认；表单用 MyGO 的 Form／Field。提供商新增编辑：保留模型记录，key 留空不变，“显示”读出已保存的 key。设备页按 Mac 分为两组，“打开”在主窗口切换到该设备。
- 自动发现：核心为每个 Harness 列出厂商安装位置（Codex 桌面应用与独立安装器、Claude 桌面应用、VS Code／Insiders／Cursor 扩展），Mac 与 Windows 都在 PATH 之后探测。win-ws 上已找到：Codex 0.155（独立安装器与 Programs 链接）、0.163-alpha（桌面应用）、0.130-alpha；Claude Code 2.1.295（VS Code 扩展）、2.1.8（Claude 桌面应用）。Codex 均不在现有 0.159 适配器范围内，需要新适配器才能导入，由用户决定。Claude Code 全局版本已按用户要求升级到 2.1.304。
- 验证：两份临时 headless 检查（未入库，真实 application、合成 ACP 运行时与远控状态），在 Mac 与 win-ws 上覆盖菜单内容、选择持久化、新会话起点、工作目录、快捷键、设置各页的增删改、删除确认（Enter 落在“取消”）、key 保留与读取、设备打开与移除，均通过。跨运行时发送被 fixture 以“不提供历史加载”拒绝，需真实 Harness 验收。双击依赖系统点击次数，仍需真机验收。

## 风险与待验证

- MyGO 尚在快速变动，需要固定版本；内存占用与 spike 的布局问题需在实际界面中复核。
- 中文输入法、长列表与长转录的性能。
- DLL 在 win-ws 上以 MSVC 构建（较慢）；Go 可从 Mac 交叉编译。
