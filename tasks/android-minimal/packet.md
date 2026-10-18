# Rust 远控库

2026-10-18，本轮 Rust Library／UniFFI 实施、静态检查、目标库和隔离验证已完成。用户最新明确边界仅限 Rust Library；Mac／Android App 接入、界面与安装不属于本轮交付，完整产品和真实网络能力仍未验收。契约归 [remote-access](../../docs/design/remote-access.md)，计划归 [plan](plan.md)，证据归 [verification](verification.md)，传输研究归 [transport-research](transport-research.md)。

## 范围与授权

Rust 提供同一设备身份、方向配对、持久联系人／pending、稳定业务服务、LAN／公网寻址、Controller 恢复和设施代际，经 UniFFI 暴露具名 API。唯一 ApplicationService／Harness 拥有业务；断连接或替换网络不取消已接纳任务，未知写入只查询原 ID，不自动重发。

本轮只交付共享 Rust packages、必要依赖接线、生成绑定／目标库、库级人工驱动和文档。允许本地提交，不 push、创建 PR、部署公网服务、读取真实凭据／配置／会话或调用真实模型；没有新增或运行自动化测试。继续使用独立 `feat/android-minimal` worktree，原目录留在 main，不改写历史。

本轮 S2／S3 的八项平台文件差异先保存于忽略的 `target/platform-scope-freeze`，再恢复到 S2 前 `01971ee`，其余原有平台实现保留。当前 App 和 `manual-android-remote.py` 尚未适配新契约，不是匹配产品。平台操作已停止：S3 APK 曾安装并打开欢迎页，现已停止；未输入邀请、配对、批准或发远程业务，未清身份／数据。隔离 Mac 进程已关闭，未覆盖正式安装；CUA 重启实例的私有环境未获验证，具体事实归 verification。

## 已完成与产物

- S1 的源码与现有条件下三轮同网生命周期完成；不是完整公网／多网络验收。S2 稳定服务检查点为 `3f36613`；S3 v2／操作作用域及范围修正为 `4821ddf`。
- S4 已实现生产 mDNS、按联系人消费与更新 locator、认证 Hello 交接、唯一有界恢复循环、主动暂停、旧意图拒绝、原 ID 自动查询、Blocked、设施设置／凭据轮换及 lookup 撤除。identity schema 1、wire／邀请 v2、普通状态 schema 3、私有网络配置 schema 1 分别表达。
- 固定 iroh 1.3 单 crate 窄补丁将独立 relay 资格与 home 候选分离；mDNS 使用每网络代际最多一份的单 worker 私有 IO runtime 完整退出。业务主 runtime 仍随稳定服务，所有 Controller 共享发现资源。两个 vendor 都有来源、许可、可重放差异与撤回条件；未再供应 swarm／acto。
- fmt、locked workspace check／全 feature 严格 clippy、remote-only 依赖／严格 clippy、Android 严格检查及两 vendor 定向检查通过。六个生产驱动、真实 Python UniFFI 的 22 项 ApplicationService＋ACP 与六项设置调用、正式受限 relay 业务、公共入口一致性、错 key Blocked、mDNS／Probe 析构门控及取消重入通过；版本边界见 verification。
- `target/s4-library-freeze/manifest.json` 记录 228 份源码摘要、构建命令和产物 SHA。host 动态库／Python／Swift 与 Android API28 arm64 remote-only release 库／Kotlin 均已冻结，生成 Swift／Kotlin 编译通过。未构建、安装或交付新的 App 包。

## 尚未证明的能力

E2 WAN→LAN→WAN、真实 NAT／UDP 故障回落、E3 蜂窝默认且 Wi-Fi LAN 保留、长时后台、IPv6-only mDNS与设施 SLA 不在本轮证据内。当前手机无蜂窝；E2 也可由独立 Wi-Fi 出口构造，E3 需要对应真实多网络环境。非空 mDNS 范围仍保留上游 wildcard 接收／发送回落，不能当作严格网卡流量白名单。

这些缺口阻止“整项产品／多路径已经验收”的声明，不触发本任务扩展到 App。后续平台消费者须独立适配当前 API，再取得真实路径和正常扫码证据；不恢复 v1 兼容层，不以 loopback、构建或临时探针替代产品验收。共享 Rust owner 已交还冻结结果，主执行者完成文档／本地提交收口。
