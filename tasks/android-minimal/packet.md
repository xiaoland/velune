# Rust 远控库

2026-10-18，用户明确纠正本轮责任边界仅限 Rust Library。平台 App 接入、界面与安装不属于本任务；此前把 Mac 本地批准页当作库验收前提的做法已停止。技术契约归 [remote-access](../../docs/design/remote-access.md)，执行顺序归 [plan](plan.md)，已有证据归 [verification](verification.md) 与 [transport-research](transport-research.md)。沿用当前任务目录，不另建竞争入口。

## 目标与授权

Rust 提供跨 LAN／WAN 的同一设备身份、方向授权、配对事务、持久联系人与 pending、稳定业务服务、统一寻址和自动恢复，经 UniFFI 暴露具名 API。唯一 ApplicationService／Harness 拥有业务，网络端点或控制端断开不得取消已接纳任务。多路径指候选、主备与自动选路，不要求带宽聚合。

仅修改共享 Rust packages、必要的 UniFFI／库构建配置、库级人工合成驱动及相关文档。允许本地提交，不 push、创建 PR、部署公网服务、调用真实模型或读取真实凭据／配置／会话；不新增或运行自动化测试。生成绑定和平台目标库编译属于 ABI 验证，平台界面与真机 App 操作不再由本任务继续执行。

继续在独立 `feat/android-minimal` worktree 工作，原目录保持干净 main。已提交的检查点不改写历史。本轮 S2／S3 的八项平台文件差异已先存为可恢复补丁，再恢复到 S2 前 `01971ee`；其余原有平台实现保留。此操作是范围收回，不表示平台实现存在已证实缺陷。

## 当前事实

- S1 当前条件下的源码和三轮同网生命周期判别完成；E2／E3 尚缺环境，不标为全部网络能力通过。设备无蜂窝；E2 未来也可用独立 Wi-Fi 出口，E3 蜂窝默认且 Wi-Fi 保留需要相应多网络条件。
- S2 已提交 `3f36613`，稳定服务、调用世代、账本和完成任务独立于网络代际。真实 ApplicationService＋合成 ACP 已验证延迟业务、丢回执、旧网络释放后原 ID 查询、真正 owner 重启与关闭 caller 取消。replace 中途取消仍只有共享实现依据，未声称行为覆盖。
- S3 Rust／UniFFI 已冻结，v2 方向授权、八位核对码、Ready／本地批准、原子状态、写前 pending、一次 Effect 作用域和准确 ID 确认均有隔离判别；三组驱动和完整 v2 Python UniFFI 合成入口通过。173 份 Rust／Cargo 源码与成功构建摘要一致，fmt、locked check、workspace 全 feature 严格 clippy、remote-only 与 Android API28 目标检查通过。生成 Kotlin／Swift 和 host ABI 构建证据保留，但不以 App 构建作为本轮出口。
- 旧平台调用方恢复后未适配 v2，不能宣称当前分支是可构建的匹配 App 产品。旧接口不恢复兼容层；后续平台接入须消费新契约。`manual-android-remote.py` 也保留旧版，当前不能用来验证 v2。
- 平台验收已停止。S3 APK 曾覆盖安装并打开欢迎页，现已停止；未输入邀请、配对、批准或发远程业务，未清身份／数据。隔离 Mac 进程已关闭，CUA 重启实例的私有环境未获验证，不宣称其隔离成功；未覆盖正式安装。具体事实归 verification。

## 下一步与责任

remote_application 保持共享 Rust／API 的唯一源码责任，主执行者负责范围、集成、文档与本地提交。平台责任暂停，禁止以库验收为由重新启动 App 工作。

先保存 S3 库检查点，再推进 S4 的库内机制：联系人 locator 合并、生产 mDNS 装配、唯一 Controller 恢复循环、活动意图输入、设施配置与串行网络代际。已定位的首个判别是配置 relay／lookup 却使其不可达时，LAN 正式入口是否仍可寻址并完成已授权业务；当前 bind 等待 online 后超时关闭整个 Endpoint，可能让公网设施故障破坏 LAN。不得用平台 UI 或额外业务连接规避。

E2 公网迁移、E3 非默认双网和完整产品交互分别保留未验证状态；库内机制完成与真实路径能力被证明分开记录。外部条件缺失不触发平台范围扩张，不放宽 10／15／30 秒判据。完成所有可独立执行的库工作后，明确交付内容与最小外部验收前提。
