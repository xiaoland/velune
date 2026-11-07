# 首个公开 Beta

## 目标与决定

用户要求发布首个公开 beta，建立版本及 release PR、无损数据迁移、GitHub Releases 自动分发 Android／macOS／Windows、develop 默认开发且 main 只接受 PR、开源许可和社区入口，并更新 README。

2026-11-07 用户确认全部方案选择，说明没有任何发布材料。采用首次启动自动备份迁移，取代开发期手动-only约束。保留 RelayKit，项目采用 AGPL-3.0-only。英文 README 配完整中文。持续授权必要实现、已确认发布流程及仓库设置。首次安装材料采用新建固定 Android 密钥，Mac ad-hoc 且未公证，Windows 暂无发布者签名，发行说明披露安装步骤。完整方案见 [design.md](design.md)。

使用合成数据与隔离入口，保留其他会话未提交工作。日常 Velune 及真实会话不参与验证。没有日常重装或退出操作。

## 当前前沿

分支 `feat/first-public-beta` 基于本机 develop 的 16 个既有未推送提交。Beta 源码尚未提交／推送，尚无公开 tag 或 Release。

- 版本、release-please、PR CI 与固定 tag 构建／打包／完整性校验已实现并通过隔离契约评审。首轮实际 CI 尚待候选提交。
- 数据 owner 和独立 reviewer 正收尾原生迁移、窗口恢复及数据读取边界。源码稳定后只运行一次 Mac dev 构建及 workspace-only 原生检查，复用已完成的 Rust 检查。
- 开源 owner 正把 Rust／Go 之外的非系统依赖源码获取和归档闭合，pipeline owner配合 source job 的必要工具链。已有真实合成 tag 归档成功，正式 tag 产物仍待 CI。
- README 与两张真实原生合成截图已完成。开发隔离、Pi 后台生命周期、日期通知主线程修复的旧有效证据复用。
- 整理稳定候选后，通过 PR 集成 develop，再提升 main、版本 PR、最终产物及发布、回流 develop。

## 一项待用户决定

旧 Android 调试 APK 与新固定发行密钥不同，无法覆盖安装。旧应用的 `noBackupFilesDir` 持有设备身份、配对和未发送内容，系统备份不包含这些状态，当前没有跨签名导入路径。

已发异步问题，等待选择补齐显式加密导出／导入后发布，或首 beta 先面向新安装、旧调试版保留原安装等待迁移。其余工作继续，发布边界依该选择收敛。

## 责任与候选边界

- Root 负责源码整合、Android密钥保管及Secrets、GitHub配置和PR/CI运营、README、最终验收与交付。
- beta_data_contract 负责持久数据转换及native owner保护，beta_data_review独立审阅正常读取／保存和恢复路径。
- beta_release_pipeline 负责版本源和跨平台发行流水线，beta_release_review的独立契约评审已返回条件通过。
- beta_oss_readiness 负责许可、署名、对应源码归档及各端入口。beta_product_screenshots已交付原生合成产品图。
- beta_candidate_gateway已纠正全局Runner缓存，恢复执行独占入口，利用配置快照及延期失效支持在途编辑，修正身份与凭据／额度退休边界。

另一条调试会话的 remote URL、临时 subscriber、Android 锁屏改动保留在工作树，候选按明确路径排除。对应8路径记录在 `target/verification/first-public-beta/candidate-paths.json`，包含其Cargo.lock/remote Cargo.toml依赖增量。构建从提交SHA取源码，不将额外dirty内容混入资产。

## GitHub 已生效设置

- 远端 develop 从原 main `ae63b280f0413c952b3054aabc2fef8627045f73` 建立，已设默认。本地先前提交及dirty保留。
- main active ruleset `main-pr-only`，id 26750418，禁止直推、强推及删除，无bypass，0个强制审批以支持solo维护，只允许merge保留提升／回流祖先关系。具名必需CI context等首个真实PR check-runs读回后配置。
- Discussions、Private Vulnerability Reporting、合并后清理分支及更新分支选项已开启，描述与topics已更新。未发送公告或对外消息。
- Actions默认token仍read，已允许bot创建PR。原生GITHUB_TOKEN bot PR CI需要维护者批准，随后其pull_request检查满足门禁。已撤销“dispatch可满足required检查”的旧方案。
- GitHub release immutability已开启并读回enabled=true。草稿阶段补齐，公开后由平台锁定资产与tag。
- 固定Android PKCS12／RSA4096密钥保存在仓库外私有发布材料目录，4个约定repository Secrets已上传并读回。密码／私钥没有进入源码、聊天或证据。公钥SHA-256为 `babd7b659b691b52ca5a7412c68f2e52abed05d0b0aeb1c56e203d548dbda4bb`。

## 证据与边界

证据根 `target/verification/first-public-beta/`。

- 四端消费纯SemVer根 `version.txt`，release-please采用standard simple，manifest仅记录工具已发布基线。SemVer／序号及iOS生成元数据、Bash/plutil窄检查通过。
- release-contract记录实际574条历史的Manifest dry run，首轮0.1.0-beta.1，后续fix/breaking beta.2。首轮提升merge commit body必须显式保留独立 `Release-As: 0.1.0-beta.1` trailer，临时文件 `promotion-merge-body.md` 已准备。五资产校验接受正确集合并拒绝9类错误。真实审批API、ruleset、平台构建及发布仍待运行。
- repository／analytics／remote无损拒写保护3+2+3检查通过，自动迁移当前6项及remote4项通过。旧serviceTier／composer输入经现有runner转换后，正常Application::open实际读回正文、附件和模型。后续native修正只补所属层检查。
- 网关回环HTTP检查覆盖在途编辑继续用旧地址/key，后续执行捕获新配置，不同runtime/model/session隔离，保存失败回滚、释放及启动失败恢复。quota fork及application lib clippy通过，详细结果见[网关任务](../gateway-versioned-runner/packet.md)。
- LICENSE与固定上游AGPL文本逐字节相同。源码helper已真实生成包含Go/Cargo vendor的合成tag归档，约82MB，524个Rust crates。Swift／Maven／NuGet／npm对应源码安排仍在收尾，未宣称全部完成。
- 两张截图由11月6日dev包真实VeluneRootView离屏渲染，使用合成内容，已目视检查。无前台激活、可见窗口或transport。截图不含系统标题栏／Scene toolbar，不充当最终Beta启动验收。详见screenshots/capture-notes.md。
- 当前150份Markdown、1035个本地路径／标题锚点通过，3个Issue YAML解析通过。最终源码稳定后刷新链接与git diff --check。

不重复完整四端交互矩阵。正式发行每个平台／架构一次最终构建，核对版本、依赖、许可、签名及核心启动路径后上传同一资产，齐备后公开。

数据独立审阅已条件通过，正常UUID-map恢复误拒由47/100降至0/100，scheduler全行预检、duplicate choices和native先持core锁均闭合。Mac SVC dev构建正在运行，随后workspace-only。Windows许可新增疑点已告知用户：预编译WinAppSDK包的Microsoft分发条款与AGPL组合义务/系统库例外需核定，advisor `beta_windows_license_decision` 正独立复核。结论前不发布Windows或半套Release，其它源码/验收继续。

Mac dev最终集成构建9c62e95b通过，前两次仅修Swift conditional-binding及NSLock返回值警告，Rust第二／三次均复用1秒内增量。workspace-only实际运行通过，覆盖多pane重开、已知旧快照备份、迁移保持mtime和两次latest选择、未来格式尝试保存字节保持、scene/device/kind/seed优先级。私有检查初次因默认TMPDIR不满足旧harness的/tmp前缀而失败，仅修合成目录位置后通过，未改生产逻辑或重复完整构建。

原生外观owner独立窄验已通过：编译实际Appearance.swift，首次load转换旧宽度、保留字号/密度与逐字节备份，后续保存有效；未知字段文件在load报告问题后再编辑仍原字节保持。证据appearance-result.log。源码归档实际增量已达166MiB，含Cargo524、Go、Swift30、Android134个source JAR、npm5个integrity包。Windows NuGet源码/许可仍未验收，因此helper在要求windows覆盖时主动失败，platform-builds依赖source gate，publish再次核对source覆盖与binary平台集合。Git仓库内嵌套解压的SOURCE-ARCHIVE provenance及真实Mac manifest子步骤均合成通过。

对应源最终细节仍在闭环：npm发布tarball完整性不等于preferred source。OSS已核实Mermaid11.12.0 min.js与npm原包一致，精确source map有1132个非空sourcesContent，新增SOURCES.json；beta_js_corresponding_source独立帮助核定5个runtime-support包中pureJS与TS源码归档需求，避免漏源和无差别归档。正式source scope尚不据此前166MiB中间包宣称全部完成。Windows源码/许可首发选择与Android数据过渡选择均等待Human。
