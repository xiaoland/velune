# Android 体验

2026-10-23 用户要求改进 Android 控制端的使用体验，重点在界面与交互：先以 HAPI、Lody 或其他同类开源移动客户端为样板模仿其界面，即使不直接使用其源码也要附参考声明，并确保开源协议与法律条款合规。来源见 [S46](../../docs/sources.md#s46)。

## 范围

- 只改 `app/android` 的界面层、构建依赖与声明文件；不改 Rust units、远控契约或业务行为
- 模仿对象是布局、信息层级与交互模式，用 Material 3 色彩角色与组件呈现（[DESIGN.md](../../DESIGN.md) 原则 1）
- 不新增附件、会话管理（重命名、删除、归档）、文件浏览、推送等 PRD 未列入 Android MVP 的能力

## 参考与许可（2026-10-23 核对）

| 项目 | 许可 | 用法 |
| --- | --- | --- |
| [tiann/hapi](https://github.com/tiann/hapi) Android（Kotlin／Compose） | AGPL-3.0 | 只看界面与截图，不复制源码 |
| [slopus/happy](https://github.com/slopus/happy) Android／iOS | MIT | 只看界面；未复制源码 |
| [LodyAI/Lody](https://github.com/LodyAI/Lody) | Apache-2.0 | 只看文档截图中的移动端布局 |
| [Innei/lody-ios](https://github.com/Innei/lody-ios) | AGPL-3.0 | 未使用 |

AGPL 源码若被复制或翻译进来，Velune 的 Android 应用就须以 AGPL 发布，因此只借鉴可见的布局与交互，代码独立编写。界面的通用模式（按项目分组的会话列表、折叠的工作过程、圆角输入框与选项行、回到底部按钮）不受著作权保护，但仍在 [app/android/NOTICE.md](../../app/android/licenses/NOTICE.md) 中注明参考来源。

仓库根目录尚无 LICENSE：在用户选定项目许可之前，Velune 自身不是开源授权状态。选择许可属于用户决定，开发方只给出建议。

## 借鉴要点

- 会话列表：按项目分组的标题（项目名、会话数），行内为运行时标识、标题、相对时间，运行中转圈、等待回应与失败用小标签，未读用圆点（HAPI、Lody）
- 会话：用户消息右侧气泡；LLM 正文全宽 Markdown；每轮的工作过程折叠为一行“工作过程 · N 个工具 · 用时”（Lody 的 “Finished working”），展开后显示思考与工具卡片；离开底部时显示回到底部按钮（Lody）
- 输入区：圆角容器，正文在上，模型选择与发送／停止圆形按钮在下（HAPI、Lody）；状态行显示连接与运行状态（HAPI）
- 审批：输入区上方的卡片，按钮并排（HAPI）
- 设备：带状态点的设备卡片与有说明的空状态

## 许可核查（2026-10-23）

- APK 内的 Java／Kotlin 依赖按 `releaseRuntimeClasspath` 核对：AndroidX、Kotlin、kotlinx、Guava ListenableFuture、AutoValue 注解、JetBrains annotations、JSpecify、ZXing 为 Apache-2.0；JNA 为 Apache-2.0／LGPL-2.1+ 双许可，按 Apache-2.0 使用；commonmark-java 为 BSD-2-Clause，autolink-java 为 MIT，其声明全文与上游逐字一致
- Rust：`scripts/android-licenses.py` 按 `remote` feature 与 Android target 的普通依赖图列出 305 个 crate，附各自随源码分发的许可文件；未带许可文件的 22 个按声明许可附标准全文。无 GPL／LGPL／AGPL；UniFFI 为 MPL-2.0，未修改，NOTICE 指明源代码位置
- 运行时缩写标识为字母，不使用厂商标志

## 待用户决定

仓库没有项目许可。若要以开源方式发布，需要选择许可；与依赖相容的常见选择为 Apache-2.0、MIT 或二者双许可（与 Rust 生态一致）。若选择 AGPL 等强 copyleft，HAPI 的代码可被合法借用，但会约束所有发行物。选定前不要宣称 Velune 是开源软件。

## 验证

按 AGENTS.md 验证原则：Kotlin 严格编译（warnings as errors）与 lint、构建 APK、安装到已连接的手机做手动检查；不新增自动化测试。真实设备配对与会话由用户验收。

2026-10-23 在已连接的 Huawei JKM-AL00a（Android 9，API 28，无动态色）上：`scripts/build-android.sh`（assembleDebug＋lintDebug）通过；安装后用 debug 的 `GalleryActivity` 截图检查设备列表、空状态、按项目分组的会话列表、新会话面板、Markdown 回复（列表、代码块、表格、链接、行内代码）、折叠与展开的工作过程、运行中状态、审批卡片与输入区；在正式入口检查“关于”与 NOTICE 显示。尚未在真实配对的 Mac 会话中检查轮询、发送中回显与回到底部按钮。

## 状态

- [x] 记录用户决定（S46、PRD）
- [x] 界面实现
- [x] NOTICE 与许可声明
- [x] 构建、安装与 gallery 手动检查
- [ ] 用户以真实配对设备验收
- [ ] 用户决定项目许可
