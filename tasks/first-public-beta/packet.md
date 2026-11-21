# 首个公开 Beta

## 目标与决定

用户要求首个公开 beta，建立版本及 release PR、无损数据迁移、GitHub Releases 自动分发 Android／macOS／Windows、develop 默认开发且 main 只接受 PR、开源许可和社区入口，并更新 README。

2026-11-07 用户确认方案并说明没有发布材料。采用首次启动自动备份迁移，取代开发期手动迁移约束。保留 RelayKit，采用 AGPL-3.0-only。英文 README 配完整中文。固定 Android 发行密钥已新建，Mac 首版采用 ad-hoc 签名、未公证，Windows 原计划为未做发布者签名的包。完整理由与新增发布范围选择见 [design.md](design.md)。

验证只用合成数据与隔离入口。日常 Velune 及真实会话保持运行。

## 当前结果

首个公开 prerelease [v0.1.0-beta.1](https://github.com/xiaoland/velune/releases/tag/v0.1.0-beta.1) 已于 2026-11-08 01:59（上海时间）发布，GitHub 已读回 immutable=true。产品源码与 tag 均为 `afd3ea74aa555f2aae00fed45b019078d9affb48`，构建号 3。

- 发布资产包含 macOS arm64／x86_64、固定密钥签名的 Android arm64 APK、Mac／Android 对应源码、四份构建 metadata、原始发行身份及 SHA256SUMS。
- [原构建](https://github.com/xiaoland/velune/actions/runs/42393786648) 的源码和两个 Mac 包通过，Android Rust／assembleRelease／lintRelease 成功，验签解析失败。[恢复流程](https://github.com/xiaoland/velune/actions/runs/42397129726) 仅补编 Android 并复用原 Mac／源码资产，四组完整门禁通过后公开。tag 与构建号始终保持不变。
- 最小实际 Runner 诊断确认 SDK 36／37 均验签成功，但 SDK 37 把证书行前缀改为 `V3.0 Signer`，原脚本自动选择预装最新版后读得空指纹。永久修复 [PR #6](https://github.com/xiaoland/velune/pull/6) 已通过 native CI 合入 develop，固定已安装的 SDK 36，保留验签日志和失败 APK。局部真实验签、YAML 与 18 个 shell 语法检查通过。
- [发布回流 PR #7](https://github.com/xiaoland/velune/pull/7) 已通过维护者批准后的原生 validate 并合入 develop，SHA 为 `8ad2dae2ff502785c568495b3813b92b0b82ff2a`。版本、changelog 和验签修正均已在 develop。
- 候选准备经 [PR #1](https://github.com/xiaoland/velune/pull/1)／[PR #4](https://github.com/xiaoland/velune/pull/4) 集成 develop，经 [PR #2](https://github.com/xiaoland/velune/pull/2)／[PR #5](https://github.com/xiaoland/velune/pull/5) 提升 main，再经 [版本 PR #3](https://github.com/xiaoland/velune/pull/3) 产生 tag。所有 main 变更均走 PR。

## 已确认的首发范围

用户已确认 Android 首版仅面向新安装，旧调试版保留，跨签名数据转移不阻塞首版。发行说明已据此更新，后续发行 APK 沿用固定密钥覆盖升级。

用户希望保留 Windows 预构建安装器，长期不以购买商业许可或签名材料为前提，目标是快速安装与低负担升级。已确认采用 Velopack，另要求 README 说明 Mac 应用使用自己的签名一键安装 iPad。

Windows 应用和发布侧已接入 Velopack 1.2.161，packId 为 Velune，渠道为 win-x64-beta。应用提供检查、下载和用户选择正常退出后安装，禁用启动自动应用。安装器、完整 nupkg 和原生 feed 会核对包内程序集／Rust DLL／转换 DLL 与 publish 内容一致。真实 SDK 的 21 项逻辑检查、18 项持久化检查和 Windows 完整 dev 构建已通过。实际 VPK 打包通过，Setup 为 115175053 字节、full 包为 107660941 字节，feed 与 payload 摘要核验通过，未安装或运行 GUI。依工具实际行为修正了渠道后缀与 delta 参数。相关提交 21e34db、c9df212 已推送至 [PR #4](https://github.com/xiaoland/velune/pull/4)，当前 CI 通过。

Windows 固定 WinAppSDK NuGet 的 System Libraries 资格及对应源码范围仍未验收。补查定位到 WinUI 真实必需的 FrameworkUdk／Windowing 运行库，官方自建 WinUI 也消费预编译组件，仅找 WinUI commit 不能闭合。用户已明确拒绝 Windows 同协议降级，要求保留六向转换，继续找兼容实现。

Bifrost Apache-2.0 固定提交 `6e640279b36337820f2088fb85f6e78f339063c3` 的隔离原型已实际通过 16 组／69 个语义条件及 12 条 C ABI 往返，包含六向请求、JSON／SSE、工具／图片／思考、数字保真、usage 存在性和终态。需 Go 1.27.3，原型由自有兼容层修复上游 usage／custom／schema 差额，共约 844 行。证据位于 `bifrost-probe/README.md`。尚未改产品转换实现或自有代码许可。

用户最终明确选择“保持现有依赖和许可，Windows 发行暂缓”。本轮不采纳 Bifrost，不改变 RelayKit／AGPL，保留 Windows 跨协议能力和安装更新准备代码。首 beta 发布 Mac arm64／x86_64、Android arm64 及对应源码。当前无需额外用户决定。

正式 workflow 已移除 Windows runner 与相关发行步骤，要求四组资产／metadata，源码覆盖恰为 macos／android。源码 helper 请求 Windows 时仍失败关闭，已收集的 22 组 NuGet／Velo 固定源码与 227 个 Windows helper Rust crate 保留供未来发行。来源 collector 已实际生成 27 个包条目和 38 份许可／notice。

## 已实现与验收

证据根为 `target/verification/first-public-beta/`。复用已有证据，只补当前决策需要的检查。

- 四端读取根 `version.txt`。release-please 管理版本 PR、changelog、草稿 tag／Release。固定 SHA 构建后核对实际二进制版本、架构、Android 证书及资产摘要，齐备后自动公开，再生成 main → develop 回流 PR。补跑沿用草稿中的原构建号。
- 实际 574 条历史的 release-please dry run 得到首轮 beta.1、后续 beta.2。发行契约接受完整正确资产，拒绝 9 类身份或集合错误，真实版本读取与 Android stale APK 选择检查通过。
- 持久数据按所属 owner 校验、备份及转换。旧格式、未知未来格式和损坏内容的保护覆盖 repository、analytics、scheduler、remote、composer、Mac／Windows 布局与偏好。合成旧 serviceTier 输入通过正常 Application::open 读回正文、附件和模型。
- Mac SVC 集成构建 `9c62e95b-69d4-4632-9e7c-6d50eaaaada1` 通过。实际 workspace-only 检查验证两组多 Tab 重开、旧格式备份、mtime 保持、latest 选择和 scene/device/kind/seed 恢复优先级。实际 Appearance.swift 独立编译验证旧宽度转换、后续保存、未知／无效输入保持原字节。
- 候选修复包含 Pi 后台任务进程寿命、Mac 日常／开发用途隔离、午夜通知主线程调度及网关执行配置快照。复用所属任务的实际关键路径证据。网关移除捕获首个调用者身份的全局 Runner 缓存，执行使用独占 Runner 与快照、延期凭据失效和串行额度写入。
- [PR #1 CI](https://github.com/xiaoland/velune/actions/runs/42381265545) 从干净提交完成 Linux 共享 core／bindings 链接、迁移与网关窄检查。[PR #2 CI](https://github.com/xiaoland/velune/actions/runs/42382182992) 通过并实际满足 main 门禁。版本 PR 的原生审批及检查链路也已走通。
- AGPL 文本、署名、社区模板、双语 README 与真实原生合成截图已整合。两张截图经实际目视检查，使用私有内容，无前台激活。
- Mac／Android／iOS 对应源码实际合成 tag 归档为 225.3 MB，包含 Cargo 524 crates、Go vendor、Swift 30 revisions、Android 134 sources JAR、npm 5 个固定包及 28 组 preferred-source 补充。归档摘要为 `50227d8a956cc75fc4bd0fe25b138756f15720a8649278de760c137572d85bbe`。Windows 二进制范围按用户决定暂缓。
- 文档 150 份、1035 个本地路径与锚点检查通过。候选 staged whitespace 检查通过。

## GitHub 与发布材料

- 默认分支 develop。main ruleset `main-pr-only`，id 26750418，禁止直推、强推及删除，无 bypass，仅允许 merge，要求真实 GitHub Actions app 15368 的 `validate`，严格要求最新基线。solo 维护无需第二位审阅人。
- Discussions、Private Vulnerability Reporting、合并后清理分支及更新分支已开启。Actions token 默认 read，允许机器人创建 PR。Release immutability 已开启并读回。
- Android PKCS12／RSA4096 密钥位于仓库外 `/Users/lanzhijiang/Library/Application Support/Velune Release/signing`，目录 700、文件 600。四项 repository Secrets 已设置。证书 SHA-256 为 `babd7b659b691b52ca5a7412c68f2e52abed05d0b0aeb1c56e203d548dbda4bb`。密码及私钥未进入源码、聊天或证据。

## 工作树与后续

当前工作树已跟随发布回流至 develop 的源码基线。保留另一条调试会话的 8 个未提交路径，未纳入候选。路径清单见证据 `candidate-paths.json`。包含 Cargo.lock、Android AddDeviceScreen、bindings remote 和 remote crate 的 URL／subscriber 调试改动。不得恢复或混入发布资产。

Windows 构建从干净提交 21e34db 建立私有 worktree，仅 target/host-rust、tools、runtime-support junction 复用已有缓存。前两次尝试停在工具准备，未进入编译，旧 SVC 16 不支持 schema 5，PyPI 没有 18.3；已把 Mac 当前使用的确切 18.3 wheel 安装至本任务私有 venv，未改 Windows 全局工具。实际 execution 53f67574-e40c-4e8e-8098-ac12d980d416 用时 275 秒，`windows-build-3.log` 记录成功。打包验证复用相同编译输入，未重编 App。已安全解除三个 junction、移除本任务 worktree／parent及远端临时 bundle、ps1、wheel，共享缓存保留，日常仓库 HEAD 前后均为 872989b7。证据为 windows-delivery-cleanup.log。

独立 owner beta_artifact_acceptance 已完成两个实际 Mac 包的 deep／strict 签名、架构、版本／构建号／源码身份、资源和动态链接检查，以及实际对应源码归档核验。Android 实际 APK 的固定证书、版本／构建号、五份 AArch64 native 库检查通过，公开 10 项资产及四个实际主资产摘要与 Release API／metadata／SHA256SUMS 一致。完整证据入口为 `target/verification/first-public-beta/artifact-acceptance/summary.md`。本轮产物检查未运行 GUI、安装 APK 或启动模拟器，源码依赖载荷复用已归档证据，未重新全量解包。发布 controller 的独立审阅通过。Mac／源码只编译一次，验签恢复仅重编 Android。

恢复 controller 精确记录于 `recovery-controller.yml`，未合入产品。两个诊断／恢复远端分支、对应本机私有 worktree 已清理，证据保留。发布与独立最终核对已完成，任务记录收敛后保留本机 develop 和另外 8 个未提交路径。
