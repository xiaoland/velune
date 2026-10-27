# 分布式可观测性

## 目标、授权与边界

为 agent 提供跨客户端、FFI、QUIC、执行与受管网关的可诊断证据，同时支持生产 OTLP 三信号。默认未配置且不启用；每个客户端独立设置，可以经独立授权一次性复制保存配置。开发入口使用私有临时覆盖，不改变普通启动偏好。使用各生态官方成熟 SDK，不引入 LiteLLM、托管后台、全局自动配置或持续同步。

用户已同意方案并明确授权实现及验收；2026-10-27 又要求“整顿、清理一下，重新启动验收；别忘了你可以自由提交”。后续持续完成既定范围并可提交本任务改动，不重复申请阶段授权。用户进一步明确直接使用已安装的 Velune、可以覆盖更新；合成数据使用独立配置目录，不再安装产品副本或编写独立配对程序。真实凭据及其他任务改动不属于本任务。已按用户要求卸载 iPad 的 `app.velune.ios` 以释放免费签名名额，并覆盖更新 `/Applications/Velune.app`。

## 当前结论

初步接入已经实现；本轮完成可用设备的关键补验并接入 SVC dev。Windows↔Android 的实际配置复制、撤销后明确拒绝与目的端原配置保持通过。Mac 和 Android 的新开发入口查到三信号，普通重启后临时配置不再生效；Windows 新入口也查到三信号，正常退出及普通重启后的双 SDK 关闭已观察。Android 的后台／强制结束／普通重启通过。已修复实际表单在本次失败时仍显示上次成功的问题，覆盖既有安装后真机复验通过。

原有真实双 SDK、FFI／QUIC／Core／Gateway、采样、代际撤销、请求结果未知、业务活跃数、三个独立 agent 诊断案例、Mac 表单保存及 Mac→iPhone Simulator 分享证据继续适用。Android 与 iPad 的真机系统证书拒绝已有证据。本轮不为未修改的 Rust 核心重复全量构建。

iPad 当前不可用，尚缺 iPad→Mac 的实际分享／撤销入口，不能宣称四平台全部验收。Mac 后续锁屏，未把 Android→Mac 配对完成误记成 Mac 实际复制。长时间移动挂起也没有观察；短暂后台和强制结束不能替代该项。

## 实现与收尾

用户明确要求接入 SVC CLI，取代此前“不引入 SVC CLI”限制；文档和任务状态仍直接用 Markdown 维护。`svc.json` 声明 sfp7 LGTM、本机及 Windows SSH、Android ADB 四项能力。SVC 管理就绪与协调，SSH／ADB／Compose 保留实际生命周期；客户端启动器只负责临时 profile、应用启动与查询，没有加入产品运行依赖或另一套进程注册表。

独立产品架构审查未发现应推翻官方双 SDK、代际撤销或独立分享授权的具体问题。独立开发环境审查发现的两个问题已修复并验证：恢复上游时复用存活的 Windows SSH master，避免争抢端口；Android 转发失败仍启动业务。Mac 开发 home 改为系统私有短路径，解决实际 Unix socket 超限；移动入口默认复用已安装应用。manifest 使用 `workspaceRevision`，明确只记录启动工具工作区，不能冒充已安装产物版本。

Android 本次 UI 修复从 `1b342eb` 的原生快照加一行修复构建，复用匹配的 Rust 库和绑定，覆盖 `app.velune.android.telemetry`；APK 与 lint 通过。当前工作树其他任务已提交 compaction，另有本地配对 CLI、重新配对修复和 Codex 调查修改，本任务不夹带或回退这些修改。Mac 安装来源仍以前轮 `4288414` 原生基线加已有 CLI 接线的记录为准，不声称整个并行工作树完成产品构建。

本轮验收脚本、运行 ID、构建来源与设备结果集中在[验收记录](verification.md)。sfp7 和本机查询通道保留供开发；测试客户端、Windows 计划任务和设备转发在收尾时停止，临时构建快照删除。iPad 既有测试包仍留有 Unset 的合成来源 profile，设备恢复后需用 `target/telemetry-native-ui/ipad-ui-config-before.json` 恢复此前无 profile 状态，并补尚缺入口。

## 环境与证据限制

当前 Android 与 Windows 可用；Mac 后续锁屏，iPad 已断开且不可用，没有可用 iPhone 真机。macOS 15.4.1 与 Xcode 26.2（包内最低 15.6），以及 iPadOS 27.0 超过其列出的 26.2 设备支持，属于官方支持范围外的组合。它是已知环境约束，尚未证明是自动化超时的唯一原因；不进行未经授权的系统升级。Apple 测试串行，复用构建，并关闭自动系统诊断收集。不得再为自有应用使用 CUA、Android／Windows 镜像串流或新的通用自动化控制器。

主要机器：sfp7 为 `sfp7-ws.localhost`；Windows 独立目录为 `C:\Users\yyh\velune-observability`；Android 真机 `HYJ4C19524006587`，既有测试包 `app.velune.android.telemetry`；iPad `4BC38BA3-E462-5248-A300-FB90F9FB0F47`，既有测试包 `app.velune.ios.telemetry`；Mac 使用 `/Applications/Velune.app`。详细运行 ID、状态、截图和原始接收数据在 Git 忽略的 target 目录，报告不输出认证值。

排查进程时曾误输出完整环境，使 NODE_AUTH_TOKEN 出现在工具输出；已通知用户并建议轮换，轮换尚未确认。后续不使用、不复述该值，只读取指定非秘密字段；启动器使用环境白名单。

## 知识与证据入口

- 已同意的 [技术方案](technical-design.md)、[实现计划](plan.md)和[验收判据](acceptance.md)。
- 当前结果、原始证据入口、已知失败与未验收项归[验收记录](verification.md)；工具与 SDK 可行性归[SDK 记录](sdk-evidence.md)。
- sfp7 版本、资源、访问策略、部署及查询实测归[部署记录](deployment.md)。
- 产品行为归[诊断 PRD](../../docs/prd/diagnostics.md#客户端遥测)，跨单元契约归[遥测装配](../../docs/design/telemetry.md)，操作入口归[开发文档](../../docs/development.md#开发遥测)，用户决定归[来源](../../docs/sources.md)。

实现与验收基线已提交：`1d4f512` 保存验收与部署材料，`08aa284` 为 Rust OTLP 基础，`1c77962` 为 FFI／远控／执行／网关关联，`4288414` 为四端原生接入和开发入口。`1b342eb` 收敛了已安装应用验收。本地配对 CLI／重新配对修复留在原任务工作树；compaction 已由原任务提交 `ba47054`。
