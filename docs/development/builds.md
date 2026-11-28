# 开发构建与缓存

[返回开发导航](../development.md)。入口归 [svc.json](../../svc.json)，平台工具链与安装前提见[平台构建](platforms.md)。

## 默认开发循环

```sh
svc run build-macos
svc run build-ios
svc run build-android
# 在 Windows 主机、仓库根目录：
svc run build-windows
```

四个入口默认 dev：Rust dev、Swift／.NET Debug、Android debug APK。Rust dev／test 默认 `debug=1`、保留增量与断言；需要变量级原生调试时显式用 `CARGO_PROFILE_DEV_DEBUG=2`，完成后恢复默认。不要为每次任务创建自定义 profile、RUSTFLAGS 或独立 target-dir。运行性能和发布质量仍使用显式 release。

对应 `build-macos-release`、`build-ios-release`、`build-android-release`、`build-windows-release` 选择优化构建。Android release 产生未签名 APK，分发签名另行处理。Mac 普通／优化开发 bundle 分别在 `target/macos-dev/Velune Dev.app`、`target/macos-dev-release/Velune Dev.app`；日常用途只由 `package-macos` 或重装入口显式生成至 `target/macos/Velune.app`。运行用途与优化 profile 独立；Windows分别是 `target/windows-dev/Velune` 和 `target/windows/Velune`。构建不会安装或退出正在使用的应用。Mac 日常重装仍通过 `svc run reinstall-macos`，明确构建 release 优化的 daily 用途后再安装、迁移和后台启动。开发启动与停止归[实例隔离](environment.md#mac-开发实例)。

每个脚本也接受显式 profile；Mac 的旧 `--build-only` 参数保留为兼容的无操作标志。需要 SDK、设备或 Gradle 参数时看平台 README。直接 Xcode 与 CLI 在同工作树并行使用的边界见[iOS说明](../../app/ios/README.md#构建)。

## 产品版本与构建号

根 `version.txt` 是产品完整 SemVer 的唯一来源。`scripts/version.py` 校验并提供完整版本、数字核心版本和发布构建号。Mac、Windows、Android 和 iOS 的用户可见版本读取该来源。Apple 平台的 ShortVersion 使用数字核心，显示字段保留预发布标识。Release Please 的 manifest 记录上一次发布的基线。Rust 内部 crate 版本独立。

`VELUNE_BUILD_NUMBER` 是共享的正整数发布序号，本地开发缺省为 1。Apple BundleVersion 和 Android versionCode 消费它。正式发行由唯一的 release workflow run 建立序号并写入草稿 Release 元数据，tag 的补跑沿用该序号。版本 helper 和该环境变量都纳入构建来源，不能把不同序号当作同一输入。可先用 `python3 scripts/version.py --format build-info` 查看，不需要构建 App。

直接 Xcode 和 iOS 构建入口复用现有首个 native phase，由根版本和模板生成 `target/ios-generated/Info.plist`。不改写已跟踪的模板，重复相同内容保留输出时间戳。

<a id="正式-beta-发行"></a>

## 预览版发行

仓库已启用 GitHub release immutability。草稿阶段补齐全部资产，公开后由 GitHub 锁定资产及关联 tag。[不可变发布](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)

发行顺序为 `develop` → `main` 提升 PR → Release Please 版本 PR → 合并版本 PR 后创建 tag 与草稿 Release → 按 tag SHA 构建并公开 prerelease → `main` → `develop` 回流 PR。热修复从 `main` 开 PR，合入后沿用同一版本与打包流程。

合并版本 PR 前，核对目标版本、`version.txt`、Release Please manifest 和生成的 `CHANGELOG.md`。需要显式选定版本时，在进入 `main` 的 Conventional Commit 正文中单独放一行 `Release-As: <目标版本>`。上次 Alpha 通过该方式指定版本。 [上游版本规则](https://github.com/googleapis/release-please/blob/main/src/versioning-strategies/prerelease.ts)

release-please PR 和回流 PR 由 `GITHUB_TOKEN` 创建。GitHub 会将 bot 创建 PR 的 `pull_request` 检查置于待维护者批准状态。维护者须在 Actions 中选择 **Approve workflows to run**，等待 `PR checks / validate` 对 GitHub 为该次 `pull_request` 事件生成的合并候选运行并通过。检查由真实 `pull_request` 事件触发，因此也满足 main 规则集的必需检查。工作流不另行派发诊断检查。Release workflow 用第一个 tag 构建 run 的 `run_number` 作为 `VELUNE_BUILD_NUMBER`，并先写入草稿 Release 的 `release-build.json`。同 tag 的 `workflow_dispatch` 补跑从该文件恢复同一序号和 SHA。补跑只接受仍为草稿的 tag。不能为补跑另起序号、移动 tag 或替换已公开资产。

PR 检查先验证版本、Release Please manifest、脚本语法和差异空白，再运行合成脚本检查。涉及 Rust 构建输入时，Linux CI 还会检查 Rust 格式、Clippy 和依赖边界，构建 `velune-bindings` 以核对共享 core 与 RelayKit 的链接，并运行默认测试。门禁范围与本地入口见[静态检查与 CI](validation.md#静态检查与-ci)。

[Release workflow](../../.github/workflows/release.yml) 配置 macOS arm64、macOS x86_64 与 Android arm64 构建，以及对应的 runner 镜像、Xcode 和 Node 版本。缓存只复用 Rust／Swift／Gradle 的构建输入产物。最终资产从固定 tag 的干净 checkout 生产。

Android 发行采用固定 keystore，由以下 GitHub repository secrets 提供材料。 `ANDROID_RELEASE_KEYSTORE_BASE64`、`ANDROID_RELEASE_STORE_PASSWORD`、`ANDROID_RELEASE_KEY_ALIAS`、`ANDROID_RELEASE_KEY_PASSWORD`。CI 要求四项齐全、APK 签名验证通过，且证书 SHA-256 与已保管发行公钥相符。不提供无签名发布回退。不要打印或提交密钥材料。

Mac 包分别为 arm64、x86_64 zip，使用 ad-hoc 签名且未公证。首次安装的系统放行步骤见[用户指南](../usage.md#平台与安装)。Android APK 可覆盖使用同一发行签名的已发布版本。不同签名的调试安装应保留，等待数据转移入口。卸载会删除私有数据。

Windows 已准备每用户 Velopack 安装器、GitHub 更新源和原生 feed，固定 CLI／SDK 为 1.2.161，应用身份为 Velune，渠道为 win-x64-beta。实际产物名称包含渠道，安装器为 `Velune-win-x64-beta-Setup.exe`，完整包为 `Velune-<version>-win-x64-beta-full.nupkg`，feed 为 `releases.win-x64-beta.json`。`scripts/package-windows-velopack.ps1` 对已构建目录打包，`scripts/package-release.py windows` 核验实际包内容和三个资产的摘要。当前正式 workflow 不调度 Windows 构建或公开这些资产，依赖与 AGPL 许可保持，跨协议能力保留。后续重新启用需先闭合 Windows 对应源码／许可门禁。应用更新入口及持久数据目录见 [Windows 说明](../../app/windows/README.md#安装与更新)。

正式包封装会读取实际二进制版本：macOS 核对 Info.plist 的 SemVer／数字核心／构建号及 daily manifest，并用 `lipo` 核对架构；Android 用 SDK `aapt` 核对 APK 的 applicationId、versionName、versionCode 与 arm64 ABI；Windows 从 `Velune.dll` 读取 ProductVersion。任一值和根 `version.txt` 或发布身份不符，封装失败。Android 构建回执根据 Gradle `output-metadata.json` 选择当前签名模式的 APK，不依赖旧文件路径猜测。

发布前核对 macOS 双架构、Android APK 和 `velune-<tag>-source.tar.gz` 及其四份 `.build.json`，确认源码 SHA、完整 SemVer、构建号、架构、签名状态和包摘要与本次发行一致。源码包包含 vendored RelayKit 源码与 macOS、Android 对应源码，`SHA256SUMS.txt` 覆盖资产清单。暂时性失败可重跑原 run 的失败 job。需要新 run 时使用同一草稿 tag 补跑。全部资产成功后解除草稿状态并建立回流 PR。

## 并行构建与结果来源

同一工作树、同一 SVC run 入口的在途请求会合并；各调用者取得同一 execution ID、输出和退出结果。已结束后再次调用会重新检查增量，不把旧成功永久当缓存。会改变构建含义的环境差异应声明在 `run.<entry>.env` 或独立入口中；只改变临时shell环境不构成SVC的新请求身份。`svc run --follow <ID>` 跟随既有执行；follower取消只退出自身等待，不终止owner构建。

项目脚本对同平台的共享生成／App输出加文件锁，dev／release也不会同时覆盖它们。跨平台保留各自输出；Cargo管理共享工具和cross域中的编译锁。安装使用独立host范围资源，不把独立worktree编译串在日常安装资源之后。没有额外的项目构建守护进程或队列。

`target/build-state/results/<platform>-<profile>.json`（Mac 为 `macos-<profile>-<purpose>.json`） 记录命令、输入前后身份、变化路径、产物摘要、耗时与退出码；成功时另存 `*-success.json`。输入身份覆盖相关源码／资源、编译配置和选择的环境摘要，并包含mtime／inode来识别构建期间临时修改后还原的情况。它是工作区本地来源记录，不是跨机器源码版本号，也不替代编译器的依赖判断。若输入变化，返回3、保留产物与此前成功记录；本次不得作为当前源码的通过证据，下一次请求使用现有增量缓存继续。

代码 Agent 的验证分工与证据复用约束见[Agent 指引](../../AGENTS.md#构建与验收协作)。

## 编译与生成的边界

| 用途域 | 路径 | 责任 |
| --- | --- | --- |
| host Rust | `target/host-rust` | Mac／Windows native，以及普通Cargo check／test／build；仓库 `.cargo/config.toml` 设置默认 |
| remote交叉目标 | `target/cross-rust` | iOS／Android，内部按target triple和profile隔离 |
| 小型host工具 | `target/build-tools` | 只依赖固定UniFFI的Swift／Kotlin／Python通用生成器 |
| C#工具 | `target/tools/uniffi-bindgen-cs` | 固定提交及SHA，匹配时直接复用，不每轮安装 |

Cargo负责Rust输入是否需重编。生成器从实际目标库读取元数据；[build-support.py](../../scripts/build-support.py)记录library、工具、配置与生成文件摘要，先在临时目录完成生成，再只更新内容变化的文件，移出不再生成的旧文件。Rust内部实现变化可能重新生成绑定，但接口没变就保留消费者文件的mtime。公开接口变化正常传递给Swift／Kotlin／C#编译器。

Android配套记录还覆盖packaged so、许可证和AAR。`--skip-native`只在现有记录与文件匹配时进入Gradle，缺失或不匹配时先拒绝。iOS native只由Xcode phase生产，声明首次生成输出；外层不再预先做同一步。

资源和App目录不先整体删除。Mac的prepared dylib在更新安装名及签名验证成功后才更新缓存标记；bundle复用已签名副本，完整App仍做最终签名和验证。构建工具内部缓存不作为产品运行依赖，bundle保持自包含。

## 空间报告与维护

```sh
svc run build-cache-report
# 仅预览；16为示例预算，应按实际热集和可用空间调整：
python3 scripts/build-cache.py prune --domain host-rust --budget-gib 16
# 确认预览后执行同一有限用途域的维护：
python3 scripts/build-cache.py prune --domain host-rust --budget-gib 16 --apply
```

`report` 另外给出用途域以外目录的合计：平台生成目录与 App 之外，任务自建的 target-dir、复制进 `target/checks` 的源码树和任务证据也在这里。本工具不回收它们；任务结束时由所属任务删除自己的输出。2026-11 的盘点中，这部分有 122 个无人引用的目录，物理占用约 86 GiB，与三个用途域之和相当。

macOS 开发 profile 的调试信息留在 `deps/*.rcgu.o`，增量重编每次以新文件名写出一整组对象，旧组不再被读取，Cargo 与 rustc 都不删除。`host-rust` 因此随编辑次数增长，2026-11 实测 20 天从约 4.6 GiB 涨到 66.5 GiB（物理），其中约 26 GiB 是无引用的调试对象，其余主要是旧变体（velune-application 一个 crate 就有 44 套）及其增量缓存；超出预算后整体回收是预期的维护方式。

普通构建不会自动清缓存。维护仅在超预算时回收所选整个用途域，下次对应构建需要重新预热；没有按mtime或Cargo内部hash猜测删除的垃圾回收器。apply先取得所有平台锁和已知Cargo缓存锁；活跃构建、外部路径、symlink／junction及含共享链接的缓存拒绝回收。下载缓存、runtime-support、可运行App、目标生成目录与用户数据不属于这些用途域。

空间统计以分配块为主，Windows缺少块统计时用文件大小。APFS 克隆共享块，`du` 与本报告都可能高估：同一批对象在一个镜像工作树中统计为 83 GiB，删除后卷可用空间只增加约 2.5 GiB。判断实际占用以删除前后的卷可用空间为准。

不要每次build先clean，也不要复制整套编译树作为任务证据。保留日志、计时、来源和必要截图；临时worktree清理前可保留有用的标准热缓存，不能覆盖其它owner正在使用的域。新机器／跨worktree优先共享下载及固定工具，生成目录和bundle不共写。
