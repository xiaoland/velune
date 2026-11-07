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

根 `version.txt` 是产品完整 SemVer 的唯一来源。`scripts/version.py` 校验并提供完整版本、数字核心版本和发布构建号。Mac、Windows、Android 和 iOS 的用户可见版本读取该来源。Apple 平台的 ShortVersion 使用数字核心，显示字段保留 beta 标识。release-please 的 manifest 仅记录它上次发布的基线。Rust 内部 crate 版本独立，不按产品 beta 逐包修改。

`VELUNE_BUILD_NUMBER` 是共享的正整数发布序号，本地开发缺省为 1。Apple BundleVersion 和 Android versionCode 消费它。正式发行由唯一的 release workflow run 建立序号并写入草稿 Release 元数据，tag 的补跑沿用该序号。版本 helper 和该环境变量都纳入构建来源，不能把不同序号当作同一输入。可先用 `python3 scripts/version.py --format build-info` 查看，不需要构建 App。

直接 Xcode 和 iOS 构建入口复用现有首个 native phase，由根版本和模板生成 `target/ios-generated/Info.plist`。不改写已跟踪的模板，重复相同内容保留输出时间戳。

## 正式 Beta 发行

仓库已启用 GitHub release immutability。草稿阶段补齐全部资产，公开后由 GitHub 锁定资产及关联 tag。[不可变发布](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)

发行权威是 `main`。日常集成继续在默认分支 `develop`。发行顺序为 develop → main 提升 PR → release-please 版本 PR → main 合并产生的草稿 tag/Release → 固定 tag SHA 的目标平台构建 → 所有资产、对应源码和校验清单齐备后公开 prerelease → main → develop 回流 PR。热修复也从 main 开 PR，按相同流程形成新 beta。Release Please 的 manifest 记录工具上一次发布版本，产品版本仍只由根 `version.txt` 提供。

release-please PR 和回流 PR 由 `GITHUB_TOKEN` 创建。GitHub 会将 bot 创建 PR 的 `pull_request` 检查置于待维护者批准状态。维护者须在 Actions 中选择 **Approve workflows to run**，让 `PR checks / validate` 对该 PR head SHA 运行并通过后再合并。检查由真实 `pull_request` 事件触发，因此也满足 main 规则集的必需检查。工作流不另行派发诊断检查。Release workflow 用第一个 tag 构建 run 的 `run_number` 作为 `VELUNE_BUILD_NUMBER`，并先写入草稿 Release 的 `release-build.json`。同 tag 的 `workflow_dispatch` 补跑从该文件恢复同一序号和 SHA。补跑只接受仍为草稿的 tag。不能为补跑另起序号、移动 tag 或替换已公开资产。

PR 检查先验证版本、Release Please manifest、脚本语法和差异空白。包含 Rust 源码或 Cargo 输入的 PR 才在 Linux 安装固定 Go 工具链并构建 `velune-bindings`，以检查共享 core／bindings 与 Linux RelayKit 链接边界。持久化文件变化再运行对应 repository、analytics、迁移或 remote 存储窄测。网关执行或配置文件变化运行其回环配置快照检查。纯文档改动不触发 Rust 构建，也不因本地未关联的全仓格式问题阻塞。

首 beta 的 Release workflow 使用 GitHub-hosted `macos-15` arm64、`macos-15-intel` x86_64 和 `ubuntu-24.04` Android arm64 runner，Windows 发行暂缓。macOS 工作流显式选择镜像内的 Xcode 26.3（Swift 6.2），满足项目 SwiftPM 依赖。runner image 更新时需确认该选择仍存在。缓存只复用 Rust／Swift／Gradle 的构建输入产物。最终资产从固定 tag 的干净 checkout 生产。历史读取 runtime-support 的 `package.json` 要求 Node.js 22.18+，工作流固定 Node 22.18.0。

Android 发行采用固定 keystore，由以下 GitHub repository secrets 提供材料。 `ANDROID_RELEASE_KEYSTORE_BASE64`、`ANDROID_RELEASE_STORE_PASSWORD`、`ANDROID_RELEASE_KEY_ALIAS`、`ANDROID_RELEASE_KEY_PASSWORD`。CI 要求四项齐全、APK 签名验证通过，且证书 SHA-256 与已保管发行公钥相符。不提供无签名发布回退。不要打印或提交密钥材料。

Mac 包分别为 arm64、x86_64 zip，使用 ad-hoc 签名且未公证。macOS 首次拦截时可在“系统设置 → 隐私与安全性 → 仍要打开”针对 Velune 放行，应保持 Gatekeeper 开启。Android beta 使用固定发行签名，首版面向新安装。现有 pre-beta/debug APK 密钥不同，无法原地覆盖，旧安装继续保留等待数据转移入口；卸载会删除私有数据。

Windows 已准备每用户 Velopack 安装器、GitHub 更新源和原生 feed，固定 CLI／SDK 为 1.2.161，应用身份为 Velune，渠道为 win-x64-beta。实际产物名称包含渠道，安装器为 `Velune-win-x64-beta-Setup.exe`，完整包为 `Velune-<version>-win-x64-beta-full.nupkg`，feed 为 `releases.win-x64-beta.json`。`scripts/package-windows-velopack.ps1` 对已构建目录打包，`scripts/package-release.py windows` 核验实际包内容和三个资产的摘要。当前正式 workflow 不调度 Windows 构建或公开这些资产，依赖与 AGPL 许可保持，跨协议能力保留。后续重新启用需先闭合 Windows 对应源码／许可门禁。应用更新入口及持久数据目录见 [Windows 说明](../../app/windows/README.md#安装与更新)。

正式包封装会读取实际二进制版本：macOS 核对 Info.plist 的 SemVer／数字核心／构建号及 daily manifest，并用 `lipo` 核对架构；Android 用 SDK `aapt` 核对 APK 的 applicationId、versionName、versionCode 与 arm64 ABI；Windows 从 `Velune.dll` 读取 ProductVersion。任一值和根 `version.txt` 或发布身份不符，封装失败。Android 构建回执根据 Gradle `output-metadata.json` 选择当前签名模式的 APK，不依赖旧文件路径猜测。

GitHub Release 中每个平台包都有 build metadata（源码 SHA、完整 SemVer、构建号、架构、签名状态及包摘要），发布前逐一比对预定清单中的实际包并核验摘要。另附全资产 `SHA256SUMS.txt` 和精确 tag 的 `velune-<tag>-source.tar.gz`（含 vendored RelayKit 源码）。首 beta 要求 Mac 两个架构、Android 和源码的四份 metadata，源码平台集合恰为 macos／android。构建失败时修复后对原来的 workflow run 重跑失败 job。需要新 run 时使用仍为草稿的 tag dispatch 重试，保持原 `release-build.json` 的构建号。全部资产成功后才解除草稿状态并建立回流 PR。首轮 promotion merge commit 的 body 须保留一次性 `Release-As: 0.1.0-beta.1` trailer（建议 subject 为 `chore(release): 提升首个公开 beta 源码至 main`，trailer 单独成行），以固定目标首版号。仅写在 PR 描述里不一定进入提交。此覆盖只用于首个候选。后续 beta 由 prerelease 策略按已合并的 Conventional Commits 递增，0.x breaking 变更按 release-please pre-major 策略递增 minor。

## 多 Agent 复用与结果来源

同一工作树、同一 SVC run 入口的在途请求会合并；各调用者取得同一 execution ID、输出和退出结果。已结束后再次调用会重新检查增量，不把旧成功永久当缓存。会改变构建含义的环境差异应声明在 `run.<entry>.env` 或独立入口中；只改变临时shell环境不构成SVC的新请求身份。`svc run --follow <ID>` 跟随既有执行；follower取消只退出自身等待，不终止owner构建。

项目脚本对同平台的共享生成／App输出加文件锁，dev／release也不会同时覆盖它们。跨平台保留各自输出；Cargo管理共享工具和cross域中的编译锁。安装使用独立host范围资源，不把独立worktree编译串在日常安装资源之后。没有额外的项目构建守护进程或队列。

`target/build-state/results/<platform>-<profile>.json`（Mac 为 `macos-<profile>-<purpose>.json`） 记录命令、输入前后身份、变化路径、产物摘要、耗时与退出码；成功时另存 `*-success.json`。输入身份覆盖相关源码／资源、编译配置和选择的环境摘要，并包含mtime／inode来识别构建期间临时修改后还原的情况。它是工作区本地来源记录，不是跨机器源码版本号，也不替代编译器的依赖判断。若输入变化，返回3、保留产物与此前成功记录；本次不得作为当前源码的通过证据，下一次请求使用现有增量缓存继续。

多 Agent 各验证自己影响的层，稳定集成后由一个owner取得共同构建；其它Agent复用这次证据。仅改UI先验证对应App；共享ABI变化才编译所有受影响消费者。不要把每个Agent的完成都变成一次全平台完整构建。

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

普通构建不会自动清缓存。维护仅在超预算时回收所选整个用途域，下次对应构建需要重新预热；没有按mtime或Cargo内部hash猜测删除的垃圾回收器。apply先取得所有平台锁和已知Cargo缓存锁；活跃构建、外部路径、symlink／junction及含共享链接的缓存拒绝回收。下载缓存、runtime-support、可运行App、目标生成目录与用户数据不属于这些用途域。

历史目录只有明确验证替代路径并列入脚本退役白名单后才能使用 `--retired <name>` 回收。旧 `target/debug`、`ios-rust`、`ios-bindgen`、`android-bindgen` 和根级Android triple已经由新链替代；旧release及仍使用的packaged输出保持受保护。空间统计以分配块为主，Windows缺少块统计时用文件大小，APFS共享块与并行磁盘活动使实际释放量不一定等于报告大小。

不要每次build先clean，也不要复制整套编译树作为任务证据。保留日志、计时、来源和必要截图；临时worktree清理前可保留有用的标准热缓存，不能覆盖其它owner正在使用的域。新机器／跨worktree优先共享下载及固定工具，生成目录和bundle不共写。
