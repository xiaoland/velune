# Windows MyGO 应用

## 当前任务

2026-10-20 用户改定 Windows 端使用 MyGO 而非 GPUI（[S43](../../docs/sources.md#s43)）。目标与 GPUI 版相同：完整端，本机运行 agents，并可控制其他已配对设备；先交付会话日常闭环（会话列表与转录、发送、停止、审批／问题），再补设备配对与设置。Go 桥坚持 purego、不用 cgo。产品约束见 [PRD](../../docs/prd/index.md)，跨单元方案见 [架构](../../docs/design/architecture.md)，远控契约见 [远程访问设计](../../docs/design/remote-access.md)。

假设（未经用户确认）：GPUI 版 `app/windows` 在 MyGO 版达到会话日常闭环前保留，之后按 hard-cutoff 删除。

## 已核实事实（2026-10-20）

- MyGO（[egoist/mygo](https://github.com/egoist/mygo)，MIT）最新发布 v0.5.5（2026-10-16），两天内连发五个版本，main 仍在更新。原生 UI 为纯 Go 自绘，组件包括 sidebar、split、list、markdown、text input、dialog 等（`docs/ui/`）。`docs/bindings.md` 只涉及 web 前端调用 Go，与 Rust 桥无关。
- 10-19 spike（同为 v0.5.5）的遗留问题：侧栏与输入区未显示、FollowEnd 未停在末尾、工作集约 295 MB（GPUI 约 64 MB）。当时判断可能是样例用法问题，未深究。
- bindings 的 FFI 面（Mac 完整 feature 生成的头文件）：约 155 个函数，即 2 个自由函数、2 个构造器、5 个 object 的 141 个方法，加上 clone／free。没有 async、callback interface，也没有 f32／f64 参数或返回值；观察者用阻塞的 `next(timeout)`。record、enum、error 与集合经 RustBuffer 序列化。

## 方案

1. **Go 生成器**：在 bindings 的生成器入口加 Go 输出，读取 cdylib 中的 UniFFI 元数据（library mode），生成一个 Go 包：每种类型的 lift／lower／read／write、object 句柄与 `Close`、错误类型。生成代码不入库，与 Swift／Kotlin 相同。
2. **Go 运行时**：RustBuffer、RustCallStatus 与调用助手。Windows 上 purego 的调用就是 `syscall.SyscallN`，只传整数寄存器，所以按 Windows x64 约定手动处理：24 字节的 RustBuffer 按值参数传指向副本的指针，按值返回由调用方在首个参数传入隐藏返回指针。先只支持 windows/amd64。
3. **冒烟**：在 win-ws 上用 Go 程序打开隔离 `VELUNE_HOME`，列出运行时与会话，并用 ACP fixture 发送一轮。
4. **MyGO 应用**：会话列表与转录、composer、停止、审批，然后是设备切换、配对（二维码、核对码批准）与设置。界面遵循 Windows 交互习惯与项目 UI 规则（双击就地编辑、消息左右布局、不显示头像与昵称）。
5. 达到会话日常闭环后删除 GPUI 版，更新 README 与架构。

## 风险与待验证

- MyGO 尚在快速变动，需要固定版本；内存占用与 spike 的布局问题需在实际界面中复核。
- 中文输入法、长列表与长转录的性能。
- DLL 在 win-ws 上以 MSVC 构建（较慢）；Go 可从 Mac 交叉编译。
