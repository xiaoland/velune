# 启动旧输入迁移修复

用户报告启动后大量 `unknown field serviceTier`，要求继续至可 dogfood。沿用完成、关键验收、提交授权；不操作鼠标、前台或重装日常应用。

## 已确认

- 新 TurnChoice 已删除 serviceTier；仓库速度模型变体改造没有覆盖旧持久化数据。
- 仅结构统计确认本机 composer-inputs schema1 有20处 null，composer-choices schema1 有10处 null；未输出正文、模型、凭据。已安装产物为 d9d2b9d dirty。
- CoreRuntime 先读取 composer-choices，故其原始反序列化错误先于 composer schema1 的迁移提示出现。

## 方案与验收

补显式升级脚本，先转换速度选择，再执行已有内容／目标拆分。不放宽反序列化，不丢输入，不猜测未知非空速度的默认模型。保留备份、锁与幂等。

使用日常已安装 dylib 的隔离合成 HOME：重现原错误 → 全迁移 runner → 核心打开／输入恢复 → 实际 Pi 回环发送／重开。应用无需为纯数据修复重编译。最后真实目录迁移需用户退出应用并确认；现有重装约定仍由用户外部执行。

分工：service_tier_migration负责迁移脚本和迁移文档；主Agent负责组合复现、关键路径、实际恢复协调与提交。

## 已完成关键验收

- 使用 `/Applications/Velune.app` 内真实已安装 dylib 生成 Python FFI，临时 HOME 和合成输入复现相同 unknown field 错误。
- 完整 runner 转换 schema1 含旧字段输入后，实际核心打开成功；输入 owner/ID/正文、运行时和 managed 模型 key 保留。
- 实际 Pi + 回环 Responses 服务发送成功；关闭应用核心后重开原合成会话，继续发送成功。未调用真实提供商或读取原生真实会话。
- 证据：`target/verification/startup-service-tier-20261104/{baseline,recovery}.log` 与 `probe.py`；无产品 Rust/Swift 修改，因此不重复整套构建或 UI 验收。
- 本机只读预览确认需处理30个空字段；当前未检测到Velune进程。已询问用户允许带备份的真实升级，待回复；不能把合成验证写成真实升级已完成。

- 持久化回归入口 `python3 scripts/check-service-tier-migration.py` 已通过5项检查：预览/备份/幂等/runner、原生非空档位和平台编码、歧义整批拒绝、重复JSON拒绝、活动锁拒绝。证据 `migration-check.log`。相对链接/锚点与 diff 检查通过。
- 最终修复仅显式升级脚本与文档，没有修改应用 ABI 或核心读取契约，不新增运行时自动迁移。

## 11月5日实际恢复完成

用户明确授权“执行迁移，你始终可以迁移”，替代本包之前等待逐次确认的执行状态。持续约定已更新到开发迁移文档与S79。

- 真实运行完整runner时暴露Codex迁移门禁：本机CLI确为0.159.3，与0.159 adapter匹配，但脚本仍要求0.166。修正为未显式指定实例时保留匹配版本；不放过适配器不匹配或明确要求迁移的错误。新增单项回归已过：完整runner继续后续输入升级、显式指定拒绝、166转换、166 adapter配159 CLI拒绝。未重复既有五项和运行时发送验收。
- 完整runner已成功处理真实目录。删除30个null速度字段、22份输入拆分到19套目标配置、7份Mac窗口输入恢复点升级、880pt外观宽度转换68.8%。原配置／输入均有备份。已不存在的旧实例关联保持原样，没有删除真实会话或猜测归属。
- 直接对照升级前备份核验：22份正文／附件／输入身份／修订、bindings、creations、附件目录、19套选择逐值一致；旧composer choices除null字段外完全一致。未输出正文或凭据。
- 已 `open -g /Applications/Velune.app` 后台启动日常应用，无前台激活或输入注入。进程存活、runtime.lock持有，45秒新增日志130条INFO、无WARN/ERROR/新问题记录；包含应用主动problems_resolved事件。启动阻碍已解除，原日志保留。没有重装应用、也没有发送真实模型请求。
- 速度备份目录：`~/.velune/before-service-tier-model-variants-ry_ddkyt`；输入备份：`~/.velune/composer-inputs.schema1.backup.json`，本机窗口和外观各有对应备份。
