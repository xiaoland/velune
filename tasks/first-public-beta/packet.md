# 首个公开 Beta

## 目标与决定

用户要求首个公开 beta，建立版本及 release PR、无损数据迁移、GitHub Releases 自动分发 Android／macOS／Windows、develop 默认开发且 main 只接受 PR、开源许可和社区入口，并更新 README。

2026-11-07 用户确认方案并说明没有发布材料。采用首次启动自动备份迁移，取代开发期手动迁移约束。保留 RelayKit，采用 AGPL-3.0-only。英文 README 配完整中文。固定 Android 发行密钥已新建，Mac 首版采用 ad-hoc 签名、未公证，Windows 原计划为未做发布者签名的包。完整理由与新增发布范围选择见 [design.md](design.md)。

验证只用合成数据与隔离入口。日常 Velune 及真实会话保持运行。

## 当前结果

- 源码提交 `6ff6c6759500eca449ce35108ad6bee1a4af2888` 已通过 [PR #1](https://github.com/xiaoland/velune/pull/1) 合入 develop，集成 SHA 为 `710418f5ad1ce2a3708821da35c493ede6d9faf1`。
- [提升 PR #2](https://github.com/xiaoland/velune/pull/2) 已通过真实 CI 合入 main，SHA 为 `03ab14a46abd7728834682b66074d9750017eaae`。合并消息保留首轮 `Release-As: 0.1.0-beta.1`。
- [Release workflow](https://github.com/xiaoland/velune/actions/runs/42382826182) 成功生成 [版本 PR #3](https://github.com/xiaoland/velune/pull/3)，目标为 `0.1.0-beta.1`。
- 版本 PR head 为 `ef6c855c269dcfefe5a8ba538d5a1ccad5c64a89`，仅变更 release-please 基线和 CHANGELOG。产品 `version.txt` 在候选中已为该版本。
- 机器人 PR 的原生 pull_request workflow 起初为 action_required。核对仓库、PR、head 和 workflow 后，使用登录维护者的 workflow approval API 成功批准。[实际 validate](https://github.com/xiaoland/velune/actions/runs/42383076285) 已通过，PR mergeStateStatus 为 CLEAN。
- 版本 PR 保持待合并。当前没有公开 tag／Release，最终发行包仍待正式 tag 构建及验证。

## 当前发布范围与待收敛方案

用户已确认 Android 首版仅面向新安装，旧调试版保留，跨签名数据转移不阻塞首版。发行说明已据此更新，后续发行 APK 沿用固定密钥覆盖升级。

用户希望保留 Windows 预构建安装器，长期不以购买商业许可或签名材料为前提，目标是快速安装与低负担升级。已确认采用 Velopack，另要求 README 说明 Mac 应用使用自己的签名一键安装 iPad。

Windows 应用和发布侧已接入 Velopack 1.2.161，packId 为 Velune，渠道为 win-x64-beta。应用提供检查、下载和用户选择正常退出后安装，禁用启动自动应用。发布包含 Setup.exe、完整 nupkg 和原生 feed，并核对包内程序集／Rust DLL／转换 DLL 与已验收 publish 内容一致。真实 SDK 的 21 项隔离逻辑检查、locked restore 和发布层合成检查已通过。Root 审阅发现持久化失败被原 owner 吞掉时更新仍会继续，app owner 正补保存结果传播；随后统一隔离 Windows 构建和实际 VPK 打包。

Windows 固定 WinAppSDK NuGet 的 System Libraries 资格及对应源码范围仍未验收。补查定位到 WinUI 真实必需的 FrameworkUdk／Windowing 运行库，官方自建 WinUI 也消费预编译组件，仅找 WinUI commit 不能闭合。用户已明确拒绝 Windows 同协议降级，要求保留六向转换，继续找兼容实现。Bifrost Apache-2.0 公共转换 API 是当前首选候选，CLIProxyAPI MIT 为备选；前者需 Go 1.27.3，并有 custom 工具、max_tokens 权威和 usage／终态适配差额。当前隔离 probe 验证六向请求、JSON／SSE，未改产品转换实现或自有代码许可。Velopack／常规 NuGet 源码 owner 并行闭合依赖材料。iPad 入口已按源码核对并补入双语 README、发行说明及 iOS 安装步骤，文档路径和差异检查通过。

现有 source helper 在要求 windows 覆盖时失败，平台构建依赖 source gate，发布器核对对应源码的平台集合与二进制集合。这一门禁在范围确定后调整。

## 已实现与验收

证据根为 `target/verification/first-public-beta/`。复用已有证据，只补当前决策需要的检查。

- 四端读取根 `version.txt`。release-please 管理版本 PR、changelog、草稿 tag／Release。固定 SHA 构建后核对实际二进制版本、架构、Android 证书及资产摘要，齐备后自动公开，再生成 main → develop 回流 PR。补跑沿用草稿中的原构建号。
- 实际 574 条历史的 release-please dry run 得到首轮 beta.1、后续 beta.2。发行契约接受完整正确资产，拒绝 9 类身份或集合错误，真实版本读取与 Android stale APK 选择检查通过。
- 持久数据按所属 owner 校验、备份及转换。旧格式、未知未来格式和损坏内容的保护覆盖 repository、analytics、scheduler、remote、composer、Mac／Windows 布局与偏好。合成旧 serviceTier 输入通过正常 Application::open 读回正文、附件和模型。
- Mac SVC 集成构建 `9c62e95b-69d4-4632-9e7c-6d50eaaaada1` 通过。实际 workspace-only 检查验证两组多 Tab 重开、旧格式备份、mtime 保持、latest 选择和 scene/device/kind/seed 恢复优先级。实际 Appearance.swift 独立编译验证旧宽度转换、后续保存、未知／无效输入保持原字节。
- 候选修复包含 Pi 后台任务进程寿命、Mac 日常／开发用途隔离、午夜通知主线程调度及网关执行配置快照。复用所属任务的实际关键路径证据。网关移除捕获首个调用者身份的全局 Runner 缓存，执行使用独占 Runner 与快照、延期凭据失效和串行额度写入。
- [PR #1 CI](https://github.com/xiaoland/velune/actions/runs/42381265545) 从干净提交完成 Linux 共享 core／bindings 链接、迁移与网关窄检查。[PR #2 CI](https://github.com/xiaoland/velune/actions/runs/42382182992) 通过并实际满足 main 门禁。版本 PR 的原生审批及检查链路也已走通。
- AGPL 文本、署名、社区模板、双语 README 与真实原生合成截图已整合。两张截图经实际目视检查，使用私有内容，无前台激活。
- Mac／Android／iOS 对应源码实际合成 tag 归档为 225.3 MB，包含 Cargo 524 crates、Go vendor、Swift 30 revisions、Android 134 sources JAR、npm 5 个固定包及 28 组 preferred-source 补充。归档摘要为 `50227d8a956cc75fc4bd0fe25b138756f15720a8649278de760c137572d85bbe`。Windows 范围仍待前述选择。
- 文档 150 份、1035 个本地路径与锚点检查通过。候选 staged whitespace 检查通过。

## GitHub 与发布材料

- 默认分支 develop。main ruleset `main-pr-only`，id 26750418，禁止直推、强推及删除，无 bypass，仅允许 merge，要求真实 GitHub Actions app 15368 的 `validate`，严格要求最新基线。solo 维护无需第二位审阅人。
- Discussions、Private Vulnerability Reporting、合并后清理分支及更新分支已开启。Actions token 默认 read，允许机器人创建 PR。Release immutability 已开启并读回。
- Android PKCS12／RSA4096 密钥位于仓库外 `/Users/lanzhijiang/Library/Application Support/Velune Release/signing`，目录 700、文件 600。四项 repository Secrets 已设置。证书 SHA-256 为 `babd7b659b691b52ca5a7412c68f2e52abed05d0b0aeb1c56e203d548dbda4bb`。密码及私钥未进入源码、聊天或证据。

## 工作树与后续

当前分支 feat/windows-beta-delivery，从 develop 创建。保留另一条调试会话的 8 个未提交路径，未纳入候选。路径清单见证据 `candidate-paths.json`。包含 Cargo.lock、Android AddDeviceScreen、bindings remote 和 remote crate 的 URL／subscriber 调试改动。不得恢复或混入发布资产。

下一步收敛 Windows 安装／更新方案和依赖分发依据，通过 develop PR 提升 main，更新版本 PR。随后合并版本 PR，跟随固定 tag 的发行构建，验证最终包和安装入口，公开 prerelease 并回流 develop。当前工作包的状态与 Android 发行说明更新保留于工作树，待下一次任务提交一并纳入。
