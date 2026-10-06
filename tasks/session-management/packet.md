# 运行时会话管理与切换加载

2026-10-06 用户授权新增删除、重命名真实 Agent 运行时会话，并修复切换加载期间选中态回跳。修改必须进入所属运行时的持久数据，不建立 Velune 标题覆盖、隐藏列表或第二套会话存储。

已观察：Mac List 使用 selectedConversationID 绑定，但 selectConversation 在打开结果完成前不设置该字段，旧 snapshot 和轮询仍可回写选中。当前后端历史入口只读；Pi SessionManager 与 Codex／DSH 需分别核对版本原生修改能力，不能将 huihua 只读能力当作修改契约。

源码唯一 owner 为 session_management，贯通 runtime／application／bindings／Mac 和必要隔离脚本。root 负责需求文档、独立验收和安装。专用 advisor 启动遇到 agent thread limit，复用熟悉现有契约的 public_model_templates 提供只读判断，不作 reviewer。guides/delegation.md 不存在，沿用明确所有权。

实施边界：目标选中、加载状态与已加载 snapshot 区分；请求代际隔离旧 poll 和迟到结果。重命名写原生标题，删除有原生确认。活动执行的安全边界、当前会话删除后的选择和失败处理随实际 API 收敛。三种固定版本逐一验证能力，有上游硬限制则明确，不冒充成功。

验证使用静态检查、类型安全、构建和临时 HOME／合成历史隔离脚本。不新增自动化测试，不读取真实配置、凭据或会话，不调用真实模型服务。完成后重建安装 /Applications/Velune.app，版本0.1 beta.1；视觉和真实体验由用户验收。

## 原生能力核对

固定版本证据（2026-10-06）：Pi 1.0.2 公共 SessionManager.appendSessionInfo 与活动 RPC set_session_name 写入原生名称；官方会话选择器删除采用 trash／unlink 单会话文件，SDK 无 delete 方法。Codex 0.159.3 生成的 app-server 契约存在 thread/name/set 与真正 thread/delete，archive 不等于删除。

DSH 0.2.0-rc.2 launcher --help 和当前 ACP 公共契约均无会话删除／持久重命名入口。npm 公共 exports 中真实存在 SessionTitleService／SessionController，重命名需要 live Session 及服务上下文装配；不能把它称为私有 API，但当前 ACP adapter 尚不能消费。未发现公开 delete service，持久层包含 generation 和 lease，单删最新 JSONL 不等于删除会话。本轮 Pi／Codex 完成原生管理，DSH 通过版本化能力明确不可用，不复制其内部存储写入逻辑；后续接入公开服务需单独设计，其限制不影响会话切换加载修复。

## 开发方隔离证据

源码 owner 已通过 debug Rust check 和 SwiftPM warnings-as-errors。`manual-session-management.py` 使用真实固定 Pi SDK／Codex app-server 验证原生名称、关闭重开持久性、原生 ID 不变、当前与非当前删除、跨实例／来源外路径／符号链接拒绝；DSH descriptor 明确不可用。Codex 名称在 native session_index.jsonl，fixed variant helper 补齐读取，huihua 包仍负责消息历史；改名不重写原消息。

`manual-session-loading.py` 编译实际 AppStore／Transport／UniFFI，临时 HOME 与合成 Pi 历史，helper 延迟装载。确认 pending 立即选择、loaded 内容保留但详情显示加载、旧 poll 不回跳、并发选择序列化、失败只恢复一次且行身份保留。均零上游请求；这不是视觉手感验收。strict clippy 与最终 clean 构建安装尚在收口，不将 debug 当作交付。

## 空草稿边界补全

收口时确认 Pi 新建会话在首 user／assistant 前不落盘，getSessionFile 返回预留路径不能证明有文件。原生 SDK 隔离试验证明 appendSessionInfo 可保留活动名称，首 user 落盘后重新打开保留名称。因此本轮继续补全当前活动、精确身份匹配、idle 且确知无文件的 Pi 草稿：重命名调用活动 RPC set_session_name 并同步；删除关闭该原生会话并清空 projection。不伪造文件，不把任意未列出的路径作为草稿。已落盘仍按来源验证和原生管理执行。此补充撤销前述源码冻结，后续静态及安装只采用完整版本。

## 完整源码冻结

草稿补充后 workspace fmt／check／all targets 与 features strict clippy、no-default bindings strict clippy 全部通过。SwiftPM warnings-as-errors、JS／Python 静态与 diff 检查通过，源码 owner 冻结。空草稿 RPC 改名和删除零请求／无文件，伪造草稿 ID 拒绝；首次 user 自然落盘、SDK.open 与 Velune reopen 同 ID／同原生名称通过。`manual-pi-native-loop.py` 因增加草稿落盘验收为六次 loopback 请求；原来源文件保持，真实服务未调用。

`manual-runtime-interactions.py` 补证执行中／审批中原生管理拒绝且审批不受影响。管理脚本补证删除失败保留当前视图、Codex 原生名称最新条目和空名称清除语义。开发方将对 clean 源码构建并安装，再从安装库复验；当前未宣称新版已交付。
