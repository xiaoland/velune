# 平台构建

[返回开发导航](../development.md)

## Mac 构建

Mac 应用的结构与界面见 [Mac README](../../app/mac/README.md)。先安装[历史读取依赖](runtimes.md)与 xURL，再通过[开发环境](environment.md)中的 SVC 入口构建或重装：

```sh
./scripts/install-runtime-support.sh
bash scripts/install-xurl.sh
svc run build-macos
# 需要更新日常应用时，在应用外执行：
svc run reinstall-macos
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

共享品牌原稿归仓库级 [assets/brand](../../assets/brand)。Mac 应用图标由 `scripts/render-app-icon.swift` 从该目录的 graphite 光学稿生成 ICNS；Dock 图标保留 macOS 的透明光学留白，绘制范围在生成器中维护，不通过修改品牌 SVG 补偿。

## iPad 与 iPhone 构建

运行 `scripts/build-ios.sh` 构建 arm64 模拟器，使用 `scripts/build-ios.sh iphoneos DEVELOPMENT_TEAM=<团队ID>` 构建设备版本。单一 Xcode target 支持 iPad 分栏与 iPhone 紧凑导航，只链接 remote-only 核心。工具链、签名、合成预览与边界见 [iOS README](../../app/ios/README.md)。

## Android 构建

`scripts/build-android-native.sh` 构建 API 28+／arm64-v8a 的 remote-only Rust so、匹配的 Kotlin 绑定与固定摘要的 rustls verifier AAR，产物在 `target/android-native` 与 `target/android-bindings`。`scripts/build-android.sh` 先执行原生构建，再构建 debug APK 并运行 Android lint；Kotlin 警告视为错误。只改 Kotlin 时可加 `--skip-native`，此时绑定必须与已有 so 配套。安装与运行见 [Android README](../../app/android/README.md)。

控制端的隔离验收用合成的完整端：Python 绑定打开临时 application、合成 ACP 运行时与公共 iroh relay，生成邀请并在核对后批准，手机侧使用真实 App。用 adb 驱动时可以临时把邀请写入应用私有目录，这类 hook 不入库。

## Windows 构建

Windows 使用 MSVC 目标（`x86_64-pc-windows-msvc`，工具链版本同 `rust-toolchain.toml`），需要 Visual Studio Build Tools、.NET 10 SDK、Go、Python 3（收集随应用分发的 Rust 许可全文）和 [llvm-mingw](https://github.com/mstorsjo/llvm-mingw)。RelayKit 桥依赖 cgo；MinGW 静态库无法可靠链接进 MSVC 产物，所以 Windows 把它构建为 `velune_relaykit.dll`，Rust 以 `raw-dylib` 绑定，无需导入库。构建脚本把 DLL 复制到 `target/<profile>/`，应用分发时须放在可执行文件旁。cgo 编译器默认为 `<arch>-w64-mingw32-clang`，可由 `VELUNE_RELAYKIT_CC` 指定绝对路径；PATH 没有传递给 build script 时，同时用 `GO` 指定 Go 的绝对路径。

先安装 runtime-support，并执行 `powershell -ExecutionPolicy Bypass -File scripts\install-xurl.ps1` 准备官方 xURL CLI。`scripts/build-windows.ps1` 先构建 `velune_bindings.dll`，再由固定提交的 `uniffi-bindgen-cs` 从中生成 C# 绑定，最后 `dotnet publish` 出自包含的 `Velune.exe`。界面、运行与已知限制见 [Windows README](../../app/windows/README.md)。

开发用 Windows 机（SSH 别名 `win-ws.localhost`）上只有一个仓库 `F:\CODING\Velune`：main 的普通克隆，origin 指向 GitHub。未推送的提交以 bundle 送达：Mac 上 `git bundle create <文件> <对方 main>..main`，复制过去后在该仓库 `git pull --ff-only <文件> main`，再删除 bundle。runtime-support、xURL 与 `uniffi-bindgen-cs` 在被忽略的 `target\` 中，新克隆或 worktree 需从已有的 `target\runtime-support`、`target\tools` 复制。并行或临时的验证副本用 `git worktree add F:\CODING\.worktrees\<任务>\velune`，用完即 `git worktree remove`。日常使用的版本先退出正在运行的 Velune，再 `robocopy target\windows\Velune %LOCALAPPDATA%\Programs\Velune /MIR` 安装，桌面快捷方式指向那里，因此重新构建不受运行中的应用影响。
