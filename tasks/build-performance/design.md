# 开发构建与缓存方案

## 判断与当前交付范围

可显著减少开发中的无效工作。主要问题是交付构建成为默认开发循环、生成输出被无条件替换、同一工具存在多份编译缓存，以及构建结果缺少供多个 Agent 共同消费的明确边界。用户已批准一并落地，实现和当前证据见[任务状态](packet.md)，实施后的操作契约归[开发构建与缓存](../../docs/development/builds.md)。以下保留调查和选择依据。

推荐沿用 SVC 管执行复用，沿用 Cargo／Swift／Gradle／MSBuild 管编译依赖。项目脚本负责 profile、native 与绑定的配套关系、稳定输出以及打包，不另建构建服务或通用调度器。

## 已确认的证据

### 时间与失效路径

- [Mac 脚本](../../scripts/build-macos.sh)无条件构建 Rust release，再以默认 dev 编译绑定生成器；删除绑定目录、Swift FFI／Swift sources 和整个 App bundle，再使用 Swift release。89 个应用 Swift 文件属于一个 target，上轮实际进入整模块编译。
- [iOS 外层](../../scripts/build-ios.sh)先调用 native；[Xcode 工程](../../app/ios/Velune.xcodeproj/project.pbxproj)又有 alwaysOutOfDate 的相同 native phase。同一次入口可以执行两次 native＋codegen。native 用独立 `ios-rust` 和 `ios-bindgen`，且删除整个生成目录。
- [Android](../../scripts/build-android-native.sh)固定 native release，并用另一份 `android-bindgen` 编译同一工具。外层提供 `--skip-native`，但调用者必须自行保证 so／绑定配套。
- [Windows](../../scripts/build-windows.ps1)固定 Rust release，每次调用固定提交的 `cargo install`，删除 C# 绑定及 publish 目录。工具安装调用本身不等于每次重新编译，但仍进入工具准备／可能联网路径。
- 生成器入口 [bin.rs](../../tools/bindgen/src/main.rs)只调用通用 UniFFI CLI，却属于依赖 application-api／telemetry 的 bindings crate。因此工具构建被业务包变化牵连，且在三个 Mac target-dir 中重复维护。
- 对相同已构建 native 库连续生成 Swift：15 文件、1,452,580 字节，内容完全相同，15 个 mtime 全部变化；首次1.348秒，第二次0.542秒。见 [实验](bindgen-observation.json)。生成器自身不是主要耗时，保持下游输入稳定更重要；本实验没有单独测量由重写触发的 Swift 重编比例。
- 既有日志：host native release 1m32s，Android native 3m25s、APK＋lint 1m18s，Windows native约9m；另一次 Clippy 显示 Cargo 锁等待。这些日志有缓存、并行及源变化差异，不作为冷／热基准或加速比例。

SVC 18.3.0 的 `svc run --help` 已承诺同一在途入口复用。安装源码 `run/runtime.py` 显示 coordination identity 为 workspace／worktree／entry，intent 摘要覆盖 argv、cwd、声明 env／env files；不包含工作树源码或任意继承环境。现有 `build-macos` 包装的 `macos-app` 是 host scope，适合日常安装排他，不适合所有 worktree 的独立编译。

### 空间（2026-11-05，只读 du）

| 目录 | 分配空间，GiB | 用途 |
| --- | ---: | --- |
| `target/debug` | 211.07 | host Rust 开发、测试、检查及历史变体 |
| `target/ios-rust` | 29.93 | iOS Rust 目标及 host 构建依赖 |
| `target/release` | 6.18 | host 优化构建 |
| `.build` | 6.06 | SwiftPM 多构建系统、配置和索引 |
| `target/ios-bindgen` | 4.76 | 第一份独立 host 生成器缓存 |
| `target/android-bindgen` | 4.69 | 第二份独立 host 生成器缓存 |
| `target/ios` | 2.92 | iOS Xcode 输出、包缓存和索引 |
| Cargo 共享下载缓存 | 2.22 | registry／git，非编译缓存 |
| `target/aarch64-linux-android` | 2.05 | Android Rust 目标 |
| Android app build | 0.38 | APK／中间产物 |
| `target/verification` | 0.32 | 当前目录内验证证据 |
| `target/macos` | 0.14 | Mac bundle |

这些是各目录实测，不是整个卷的完整盘点；APFS clone 共享块的可回收量不能由 du 精确推算。Cargo host debug 才是当前空间治理第一项，不能用清理截图或下载缓存替代。`.build/apple` 约2.18GiB，`index-build`约1.77GiB，旧 architecture build约1.57GiB，开发入口混用不同 backend／profile 会保留多套产物。Rust debug 分项为：incremental 138.19GiB、deps 71.00GiB、build script 输出0.38GiB。增量目录有2,161个 crate/hash 子目录，涉及38个crate，application341个、bindings312个、gateway212个；这些是缓存变体，不等于活跃进程或简单的构建次数。

额外抽样 application 的435份 lib／test-lib fingerprint：只有一个rustc身份、一个rustflags集合和一个config身份，但存在6个profile摘要、3组features、6个源码path摘要和311组依赖图摘要，见[样本](fingerprint-observation.json)。这支持优先统一编译用途与依赖图、减少临时构建配置并管理历史缓存；不能把全部积累归因于编译器版本变化，或只关掉增量。未逐一反推每个历史变体的命令。

## 候选比较

| 路线 | 普通开发一次变化的后果 | 判断 |
| --- | --- | --- |
| 继续完整 release，仅增加编译缓存／更多并发 | 不解决输出重写与整模块优化；增加内存和磁盘压力 | 不优先 |
| SVC 合并在途请求＋原生增量工具＋稳定生成输出 | 相同平台同配置只有一个执行；无变更阶段廉价确认，变更沿依赖下传 | 推荐 |
| 自建 daemon、全图调度及按每次源码摘要建立产物库 | 要负责取消、失效、跨平台身份和缓存淘汰；源码频繁变动产生更多存储 | 当前收益不足 |

## 推荐职责与操作

### 1. 统一开发与发布的 profile

开发 App 使用 Rust dev 与 Swift／.NET Debug，Android保持debug APK；release仅用于发布、性能判定及明确要求的交付。Rust dev 保留增量，建议默认 `debug=1`（limited），避免完整类型调试信息与多套变体长期积累；需要变量级原生调试时明确使用完整信息。选择 `dev` 而非再造一套常驻 `dev-fast` target cache。保留检查与断言，不以更改业务 feature 取代优化。

上述符号级别及运行表现需通过小样本确认后成为默认。dev 会牺牲部分运行性能，不能在未观察前承诺替代性能验收。Cargo 的 profile／incremental/debug 含义以[官方 profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)为准；Swift 增量与 whole-module 差异见[官方 driver](https://github.com/swiftlang/swift/blob/main/docs/Driver.md)。

### 2. 稳定 native→绑定→App 的生产链

Cargo 仍判断 Rust 输入是否需要编译，不在脚本中另写一套按源码后缀推断依赖的规则。工具链、target、features、profile及关键编译环境形成明确 BuildSpec，脚本不依赖各 Agent 临时设置不同 RUSTFLAGS／wrapper。

把通用 Swift／Kotlin CLI 移为只依赖固定 UniFFI 的小工具，共享一份 host 工具产物；Windows的固定C#生成器改为版本明确的 ensure，存在且版本正确时直接运行。工具准备从每个App构建中抽成可复用步骤。

绑定与所读取 native 库配套记录工具／配置及输入摘要。生成到临时目录，与稳定消费者目录逐文件比较，只有变化才替换，同时移出已不再生成的旧文件。内部 Rust 实现变化但接口不变时可以重新生成，再保持 Swift／Kotlin／C# 输入不变。无需先引入第二套 ABI 模型。失败不发布半套 native／绑定结果。

应用不再先删除全部输出；编译器和增量同步处理变化。资源、许可证、图标按实际输入更新。iOS只保留一处native生产责任，CLI与Xcode入口调用同一幂等步骤，正确声明其输出；不能让Xcode直接构建绕过native准备。安装仍独立于开发构建。

### 3. 用 SVC 复用工作区内的执行

各平台提供标准开发入口，同一worktree／平台／配置的并行请求加入同一在途执行；已结束请求再次调用时仍让原生构建工具确认增量。日常安装独立保留host级排他。无需在Velune内再实现一个后台构建队列。

SVC的“同一请求”不意味着源码一致。脚本记录相关输入在开始／结束的身份及实际产物：若构建期间输入已变，本次结果不冒充最新源码的验收依据；下一次请求继续现有增量缓存。等待者消费同一结果／退出码，但仍核对它对应的输入。该记录用于结果来源，不替代 Cargo／平台编译器的失效判定。

共享单位是固定编译用途的热缓存和同一在途执行，不是每个Task一份target，也不是每次source hash一套永久目录。不同平台的native库不能互换。按host本地、remote交叉目标及小工具划分必要的缓存域；保留目标triple差异，避免一个大锁阻塞所有工作，也不无限复制每个平台的host依赖。第一步保留host、remote交叉目标及host工具三个用途域，remote域内部保留各triple；不按Task增加缓存根。若测到交叉目标的锁等待再决定独立lane，接受少量host依赖重复以换取确有收益的并发。跨worktree优先共享下载与固定工具，不直接混用可写的生成／bundle输出。

多 Agent 的验收由集成owner在稳定输入上取得一次共同构建；各 Agent 先做自身影响层的检查，再共享同一最终构建证据。修改Kotlin／Swift不会自动触发四端完整验收；共享ABI变化才完成所有受影响绑定／消费者编译。

### 4. 空间治理与速度一起落地

先减少会再次生成的重复物，再回收明确被新布局替代的旧目录。首批候选是多份完整bindgen缓存、废弃Swift backend／配置、Rust debug中的不再使用变体。当前活跃编译目录不清理；也不逐文件按mtime猜Cargo内部依赖。

区分三类：下载／固定工具共享缓存，当前配置的可复用编译缓存，以及可运行App与任务证据。只保存每个平台当前和上次成功产物作为发布／回滚需要；证据以日志、测量及必要截图为主，不把复制出的整套构建树长期当证据。提供只读空间报告和明确路径的回收预览；按目录类型及活跃执行约束回收，不使用周期性全量cargo clean。

统一入口不能阻止编译器内部历史变体长期增长。建议维护入口按用途域报告大小和活跃执行，在无构建时轮换超预算的完整缓存域并只预热标准配置；不自行解析Cargo内部hash来删除“看起来旧”的文件。预算按新路径验收后的热集大小确定，超过预算时展示回收量和一次冷重建代价，维护与普通构建分开。首轮优先替换这211GiB旧host debug域，并退役重复bindgen域。

旧cache的一次性回收应在新路径构建通过且无人使用后执行。`du`给出候选大小，回收前后用卷可用空间与后续热构建成本判断实际效果，不能承诺等量释放。

## 实施与验收顺序

1. 先Mac开发链和通用host生成器：量一次基准；开发profile、稳定绑定、增量bundle、SVC复用共同落地。避免先花时间构建四套新缓存。
2. 同一套输入／配套约束接到iOS、Android、Windows；消除iOS双调用，保留原发布／安装行为。
3. 验证已替代缓存的回收预览，维护完成后执行定向回收，再量热路径和空间。

核心观察为：无变化第二次构建；仅改UI；改Rust实现而不改接口；改公开接口。复用同一个最终App验证合成启动／编辑／本地核心调用；并行两个相同请求只生产一次且同报结果，过程中源码变化不会获得虚假的“最新通过”。只在新改变失效的层补验，不重复既有全套矩阵。初次验证按稳定输入执行，才比较壁钟时间、编译器调用数、锁等待与目录／卷空间。尚不承诺具体加速百分比。
