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

对应 `build-macos-release`、`build-ios-release`、`build-android-release`、`build-windows-release` 选择优化构建。Android release 产生未签名 APK，分发签名另行处理。Mac 开发 bundle 在 `target/macos-dev/Velune.app`，release 在 `target/macos/Velune.app`；Windows分别是 `target/windows-dev/Velune` 和 `target/windows/Velune`。构建不会安装或退出正在使用的应用。Mac 日常重装仍通过 `svc run reinstall-macos`，明确构建 release 后再安装、迁移和后台启动。

每个脚本也接受显式 profile；Mac 的旧 `--build-only` 参数保留为兼容的无操作标志。需要 SDK、设备或 Gradle 参数时看平台 README。直接 Xcode 与 CLI 在同工作树并行使用的边界见[iOS说明](../../app/ios/README.md#构建)。

## 多 Agent 复用与结果来源

同一工作树、同一 SVC run 入口的在途请求会合并；各调用者取得同一 execution ID、输出和退出结果。已结束后再次调用会重新检查增量，不把旧成功永久当缓存。会改变构建含义的环境差异应声明在 `run.<entry>.env` 或独立入口中；只改变临时shell环境不构成SVC的新请求身份。`svc run --follow <ID>` 跟随既有执行；follower取消只退出自身等待，不终止owner构建。

项目脚本对同平台的共享生成／App输出加文件锁，dev／release也不会同时覆盖它们。跨平台保留各自输出；Cargo管理共享工具和cross域中的编译锁。安装使用独立host范围资源，不把独立worktree编译串在日常安装资源之后。没有额外的项目构建守护进程或队列。

`target/build-state/results/<platform>-<profile>.json` 记录命令、输入前后身份、变化路径、产物摘要、耗时与退出码；成功时另存 `*-success.json`。输入身份覆盖相关源码／资源、编译配置和选择的环境摘要，并包含mtime／inode来识别构建期间临时修改后还原的情况。它是工作区本地来源记录，不是跨机器源码版本号，也不替代编译器的依赖判断。若输入变化，返回3、保留产物与此前成功记录；本次不得作为当前源码的通过证据，下一次请求使用现有增量缓存继续。

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
