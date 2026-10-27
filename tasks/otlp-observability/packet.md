# 分布式可观测性

## 目标、授权与边界

为 agent 提供跨客户端、FFI、QUIC、执行与受管网关的可诊断证据，同时支持生产 OTLP 三信号。默认未配置且不启用；每个客户端独立设置，可以经独立授权一次性复制保存配置。开发入口使用私有临时覆盖，不改变普通启动偏好。使用各生态官方成熟 SDK，不引入 LiteLLM、托管后台、全局自动配置或持续同步。

用户已同意方案并明确授权实现及验收；2026-10-27 又要求“整顿、清理一下，重新启动验收；别忘了你可以自由提交”。后续持续完成既定范围并可提交本任务改动，不重复申请阶段授权。开发只使用合成数据和隔离安装；日常客户端、真实凭据及其他任务改动不属于本任务。唯一已执行的日常安装变更是用户明确要求卸载 iPad 的 `app.velune.ios`，以释放免费签名名额；不得声称所有日常安装均未变化。

## 当前结论

四端实现已装配，尚不能宣称完整验收通过。既有真实双 SDK、FFI／QUIC／Core／Gateway、采样、撤销、请求结果未知、业务活跃数和三个独立 agent 诊断案例已通过。Windows↔Android 原生双向配置复制已通过，撤销后的明确界面拒绝尚缺；锁屏前 Mac→iPhone Simulator 的复制／撤销观察可复用。2026-10-27 清理后重启验收，Mac 的实际表单保存／正常退出／普通重启、Mac／Windows 两个独立进程的分享对角项及 Mac→Windows 跨机复制均通过。Android 与 iPad 真机的实际双 SDK 三信号、自签证书拒绝及隔离配置恢复也已补验。Windows→Mac 首次配对返回 Unauthorized，正在定位产生层；未将此格记为通过。

验收对象是产品行为。框架名称、运行器启动、授权探针、可读控件或按钮调用成功不构成产品通过。每项观察必须关联实际实例、私有目录和构建，并验证保存／生效／导入／业务终态等最终效果。用户进一步明确直接使用已安装的 Velune、可以覆盖更新，并指出 ProfilePair 辅助程序造成打扰。已停止／移除该路线。验收按四端实际设置入口和每端至少一次真实来源／目的端覆盖，公共协议边界复用既有证据；不再为凑 16 格新增辅助程序，不声称未观察组合均实测通过。

## 当前前沿

1. 清理已完成：停止本任务隔离客户端，移除 Windows 测试计划任务、Android 辅助测试 APK、ADB reverse、Windows 临时隧道、iPad UI runner 和五份重复 Apple DerivedData；当前没有运行中的模拟器。sfp7 服务、Mac 查询隧道和唯一证据保留。
2. Mac 保存悬案已用新的真实输入／提交／重启证据补足；旧直接 AXValue 检查的唯一失败原因未独立复现，不归因为产品缺陷。短用例归 [mac-ui.py](acceptance/mac-ui.py)，只控制 `app.velune.telemetry`，约 5 秒完成，不启动模拟器。
3. 按现有 build／install 路径更新正式 Mac Velune，单 worker 构建、沿用现有开发签名；后续使用已安装应用与已有本地配对 CLI／原生 UI 自动化。剩余重点是 iPad→Mac 的实际分享、撤销拒绝与保留配置闭环；不再追查辅助程序才出现的错误，除非真实 App 复现。移动真机网络／系统证书已通过，后续补必要的实际生命周期。充分的共享端到端证据不重跑。
4. 提交按本任务归属拆分；工作树中的本地配对 CLI、重新配对修复和 Codex 调查属于其他既有任务，不夹带或回退。混合文件需逐段核对依赖，不能用整个工作树提交制造“干净”。

## 环境与证据限制

Mac 与 Android 已解锁，iPad 真机已连接且 Developer Mode 开启；当前没有可用 iPhone 真机。macOS 15.4.1 与 Xcode 26.2（包内最低 15.6），以及 iPadOS 27.0 超过其列出的 26.2 设备支持，属于官方支持范围外的组合。它是已知环境约束，尚未证明是自动化超时的唯一原因；不进行未经授权的系统升级。Apple 测试串行，复用构建，并关闭自动系统诊断收集。不得再为自有应用使用 CUA、Android／Windows 镜像串流或新的通用自动化控制器。

主要机器：sfp7 为 `sfp7-ws.localhost`；Windows 独立目录为 `C:\Users\yyh\velune-observability`；Android 真机 `HYJ4C19524006587`，隔离包 `app.velune.android.telemetry`；iPad `4BC38BA3-E462-5248-A300-FB90F9FB0F47`，隔离包 `app.velune.ios.telemetry`；Mac 隔离包在 `target/telemetry-native/mac/VeluneTelemetry.app`。详细运行 ID、状态、截图和原始接收数据在 Git 忽略的 target 目录，报告不输出认证值。

排查进程时曾误输出完整环境，使 NODE_AUTH_TOKEN 出现在工具输出；已通知用户并建议轮换，轮换尚未确认。后续不使用、不复述该值，只读取指定非秘密字段；启动器使用环境白名单。

## 知识与证据入口

- 已同意的 [技术方案](technical-design.md)、[实现计划](plan.md)和[验收判据](acceptance.md)。
- 当前结果、原始证据入口、已知失败与未验收项归[验收记录](verification.md)；工具与 SDK 可行性归[SDK 记录](sdk-evidence.md)。
- sfp7 版本、资源、访问策略、部署及查询实测归[部署记录](deployment.md)。
- 产品行为归[诊断 PRD](../../docs/prd/diagnostics.md#客户端遥测)，跨单元契约归[遥测装配](../../docs/design/telemetry.md)，操作入口归[开发文档](../../docs/development.md#开发遥测)，用户决定归[来源](../../docs/sources.md)。
