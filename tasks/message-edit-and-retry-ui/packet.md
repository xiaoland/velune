# 任意消息编辑与自动重试界面

用户授权恢复任意用户消息编辑后重新发送，并为可重试错误统一呈现实际错误、重试进程／状态，覆盖 frequency limit429 与 server overloaded。延续分层、关键路径验收；未授权提交、日常重装或真实数据／模型调用。

## 实现与边界

既有 editing.rs 排除了所有 logical_projection，连单目标别名也失去资格；Mac 另因下一轮暂选运行时不同隐藏编辑入口。已修复两处：简单关联操作原生目标、原地继续保留公开身份；重新发送使用会话当前上下文运行时并显示目标，编辑失败不关闭窗口。

Codex 0.159.3 的官方 ThreadRevertParams／ThreadForkParams 只有轮次边界，不能单独替换同 turn 的 steer。用户已要求任意消息，既有 PRD 允许新会话继续，故对无精确原生边界的 steer／关联前段复用该入口：仅新会话，在当前 Codex 创建独立持久线程，以 developer instructions 提供消息之前的可读取参考上下文。所选及后续消息不进入引用；不重放工具／审批，不共享旧原生线程所有权。曾询问此体验偏好，未收到答复；上述为在既有授权内的实现判断，不记录为用户额外批准。只读子 Agent、运行中／待审批、上下文无可靠分叉机制的其他 Harness 仍受现有能力限制。无持久 schema 变动，无迁移。

重试复用 RuntimeFailure／RuntimeRetry：Mac 新增 RuntimeRecovery，转录区底部集中显示原始错误、等待倒计时／次数、原生未知进度、停止／耗尽／恢复结果，避免折叠后不可见或重复活动行。composer 高度不变。Pi auto_retry_start 改为同一类型化状态；原生结果仍是终态权威。Codex 旧客户端所需的 Native 文案在结束时清除，避免过期。其他平台此次未增加专门恢复 UI。

产品事实归 [composer PRD](../../docs/prd/composer-actions.md)，通用自动恢复模式归 [DESIGN](../../DESIGN.md)，实现说明归 Mac／agent-runtime README，来源 S65。没有提交、日常重装或真实数据／供应商调用。

## 分层证据

- 基础：两库 release strict clippy 通过 `/tmp/velune-message-retry/clippy-delivery.log`；只运行相关 Codex 恢复协议测试（1 项），覆盖空输入、原生重试优先、429、停止及过期说明清除，`retry-final-test.log` 通过。Pi 原生终态局部测试见 `pi-retry-test.log`（1 项）。
- 原生组合：实际 Codex 0.159.3＋application＋loopback Responses，全合成 HOME。`edit-e2e-3.log` 验证首条 fork、单目标关联中间消息原地替换、公开身份及新 application 读回；`reference-e2e.log` 验证真实 steer、新独立线程、引用截断、源原生文件字节不变和重开后继续保留引用。
- 跨运行时前段补验发现读取时传错 source/context 运行时，已改用 source 读取；`reference-mixed-e2e-3.log` 通过（数据根 `/tmp/velune-reference-edit-yacaksgi`），证明当前上下文运行时创建新线程、引用不含所选及后续消息。前两次失败分别为合成控制权等待缺失、实际运行时参数错误，不算通过证据。
- Mac：先前独立副本已观察 429 倒计时、停止结果、原生过载不伪造次数、耗尽状态及单行输入区。发现重复旧活动行与预览 turn 状态不一致，已修复；最终构建后只补更新后的恢复条及真实历史消息编辑入口，不重复旧布局矩阵。
- 最终原生 Mac 补验通过：`VeluneRecoveryFinal.app` 的 429 恢复条显示实际 HTTP 错误、倒计时、第 2/5 次、停止及完整错误详情 popover；完整区域截图确认没有旧活动进度重复、单行 composer 独立且无溢出。实际合成历史中右键较早消息打开编辑窗口，目标显示当前 Codex B，可修改原文；确认只含新会话继续及参考上下文说明，取消确认仍保留修改。另一个普通较早消息确认同时提供原地／新会话继续。GUI 未真正派发模型请求，实际发送及持久化复用上述原生 E2E 证据。全部通过 CUA 原生应用操作，没有 Windows App／远程桌面验收。

## 当前收尾

增量 SVC build-macos `d3e56a1c-0a53-437d-a206-58436e1a8a8b` 通过（`/tmp/velune-message-retry/build-delivery.log`）；两库严格检查、跨运行时补验、文档相对链接／锚点及 diff 检查通过。锁屏解除后已完成上述最小 UI 补验，旧副本和最终独立副本 `/tmp/velune-message-retry/VeluneRecoveryFinal.app` 均正常退出。临时 PATH wrapper 使用已固定版本的 UniFFI generator 避免无关 CLI 重编译，构建和资源仍由 SVC 执行。日常 Velune 未退出／覆盖，没有提交；当前实现与验证工作已完成。
