# Velune：收敛 Harness 路由与跨会话协作架构

- **目标**：为 Velune 固定的 Codex、Claude Code、Pi 设计多订阅／多供应商自动 LLM 路由、持久任务—会话模型和跨会话协作；方案由设计方收敛、用户复核，不再以现成支持度重选 Harness
- **输入**：[PRD](../../docs/prd/index.md)、[设计委托 S9](../../docs/sources.md#s9)、[复核与技术方向 S10](../../docs/sources.md#s10)
- **状态**：2026-10-01 11:28 用户认可核心产品设计与原生技术方向；已更新 Rust／SQLite、客户端边界与协作机制，具体原型仍未授权／未运行
- **交付**：[架构方案](../../docs/design/architecture.md)为设计归属，[证据快照](evidence.md)保留版本、链接、事实与来源限制
- **授权边界**：本地文档与 Git 提交；不接管真实凭据、不调用模型、不安装运行 Harness、不编码产品、不部署；Factory26 保持停止。SVC 仅采用文档方法，无 CLI。远端发布仅以用户另行明确授权的 Velune 文档上传为限
- **研究完成判据**：三 Harness 都给出可执行路径和限制；自动账户／模型选择与订阅约束置于核心；控制、数据、身份、恢复与通信有契约；推荐和未知项分开；给出原型验证，不冒充已跑通

## 已确认方向与细化推荐

1. 一个本地控制服务＋三原生 adapters；一套路由策略，Codex Responses、Claude Messages、Pi virtual-model hook 分别执行
2. Account／workspace／entitlement／凭据绑定独立；账户选择输出 dispatch／wait／handoff／needs_decision，不建共享订阅 token pool
3. 稳定逻辑 Session＋不可变原生执行绑定 SessionSegment＋可替换 Run；兼容无状态请求可在 Segment 内换上游，原生身份切换自动安全续作，保持消息地址和任务树位置
4. 已明确认可 MCP 协作工具、ACP 可替换接入；协作由 Coordinator＋Broker／Supervisor／Adapter 的发现、注入、委派和回传完成，持久状态只提供可靠性
5. 已确认 Rust＋SQLite、Apple Swift／UIKit 方向及 Android Kotlin／Jetpack Compose 原生 UI；先本地核心，后远程 Runner。Rust Host／Client 分层、薄 SDK 桥、Mac Catalyst 与 UniFFI 为细化推荐

用户认可的是产品与技术方向，不自动批准所有详细契约或产品实现，也不证明真实账户组合已获准／兼容。

## 关键新证据

- OpenAI 已有正式 SIWC ChatGPT 计划接入，适用于符合条件的开源／本地应用；商业／托管走单独资格路径。原生 app-server 旧认证不能当商业托管授权
- Claude Code 允许平台运行原版 binary 并让用户通过官方流程登录；网关技术上可传原生 OAuth，但凭据中介限制仍在。API 及原生订阅不混为同一路径；非 Claude gateway 兼容需要实验
- Pi 当前源码有逐请求 virtual models，RPC／MCP 能力比旧版本描述丰富；但没有内建沙箱或逐工具审批
- ACP 当前 v1 有可选 list／resume 等，不能说它没有会话发现；也不能将其解释为任意活动进程／全局任务控制
- `lexoliu/acphub` 的精确仓库和 README 在授权 GitHub API 返回 404；未获得源码／commit，不能判断私有、改名或不存在。指定参考的源级检查仍未完成；主方案不以它为依赖

详情与版本均在[证据快照](evidence.md)，此处不复制完整研究报告。

## 原型验证队列

当前**全部未运行**。先用本地假服务／合成状态，不用真实账户／付费调用；注册、登录、密钥配置、安装与模型成本测试另获授权。验证失败用来定位适配问题和修正实现，不用于删除已固定的 Harness。

| 验证 | 输入／方法 | 需要保存的证据与通过条件 |
| --- | --- | --- |
| P1 全请求路由覆盖 | 三 Harness 的 main、subagent、压缩、标题／辅助调用走假上游；锁 binary／SDK／provider 版本 | 请求清单、实际控制粒度、归属 ID 与遗漏清单；不得把未受控辅助流量标为受控；native 订阅与 API lane 分别报告 |
| P2 流／工具协议保真 | Responses／Messages fixtures：并行 tool IDs、半截参数、推理块、图像、cache、ping、未知字段、错误 | 原生事件与重放 trace；有序正确终止、不重复工具、未知语义显式拒绝；跨协议组合单独判定 |
| P3 账户与预算 | 两个合成账户／workspace、多 worker 并发、未知配额、局部限流、重复 refresh、未授权 API fallback | 身份不混用、原子 slot／Attempt 预留、不把未知费用归零；无授权不付费／不出站；真实权益之后单独验证 |
| P4 自动订阅续作 | 当前原生账户不可调用，候选账户获准；源工具完成／未完成／状态未知三种情况 | Router 输出与 Supervisor trace；完成状态才自动建立新 Segment，未知时不双跑；稳定 Session 地址和旧证据仍在 |
| P5 重试与副作用 | 首字节前后断流、上游已接受但无回执、宿主工具副作用、SDK 多重 retry | 不拼接不同模型流、不盲重发不确定操作；每次尝试记账并独立占用预算；首版上游变更型工具关闭 |
| P6 持久恢复 | DB 写入／native submit／ack 之间逐点 crash；事件间隙与重连 | injecting／delivery_unknown 被保留；能核对则恢复，不能核对不自动重投；任务／消息不伪完成 |
| P7 控制与审批 | 双客户端抢 lease、旧 epoch 响应、审批断连、一次许可与持久设置混淆 | 单控制者、旧审批失效、未知审批 fail-closed；无 native fencing 时不得强抢 |
| P8 三 Harness 协作 | A 发现 B、发出委派，B busy／idle／断连，结果回到 A；重复消息、TTL、互等循环 | 原生注入／实际消费和结果关联 trace；仅 DB 接纳不能算协作成功；正确权限、去重、unknown 语义与产物，失败不静默改派重做 |
| P9 终止与后台工作 | Pi queued follow-up 后 Stop；Claude result 后仍有 background work；Codex turn/completed | cancelCurrent 与 cancelAll 分开；正确等待 settled／会话状态；不把首个结果当整个任务完成 |
| P10 执行与凭据隔离 | 工具访问凭据目录、环境、控制接口、修改 hooks、加载项目扩展／MCP；并发 worktree 写冲突 | TCB 不可修改／越权读取，票据 scope／epoch 正确，子进程不继承控制权；不达标不能宣称安全受管 |
| P11 路由效果 | 固定合成或获准任务集与固定模型基线 | 成功率、测试质量、修订次数、延迟、实际 API 成本、订阅等待与恢复成功；不用未授权真实代码做多模型 shadow |
| P12 Rust／SDK 桥 | Rust 驱动三 Harness，Pi 每请求路由 hook，Claude 官方 SDK lifecycle／审批，桥崩溃／背压 | 领域规则只在 Rust；桥不自行路由或记账；原生 ID／事件／取消保真，SDK 版本可锁定 |
| P13 原生 UI／FFI | Swift UIKit／Mac Catalyst 候选与 Kotlin Compose，Rust Client 异步事件／取消／释放／重连 | 主线程正确、无释放后回调、错误类型保留、事件有背压；UniFFI 或替换桥不改变契约；不含 Web UI |
| P14 服务与移动生命周期 | 桌面关 UI、Host 重启；移动挂起／断网，离线命令与过期审批 | 任务不随 UI 消失；Host 权威、缓存非主库；未接纳命令不显示运行，旧审批不重发；手机不承担默认常驻 Harness |
| P15 Apple 桌面边界 | Mac Catalyst 菜单／窗口／键盘／文件交互，Rust 服务启动／IPC／sandbox 与分发路径 | 形成 Catalyst 可行证据及真实限制；需要 AppKit 时明确提出范围变更，不把 Apple-universal 当全平台已完成 |

## 仍需外部输入或后续验证

- **细化选择**：Apple 具体平台范围、Mac Catalyst／必要 helper 是否够用、FFI／官方 SDK 薄桥与打包；提出推荐并用原型核对，不重开已确认的 Rust／SQLite／原生 UI 方向
- **部署资格**：若产品最终商业／远程托管，核对 SIWC 合作资格和 Claude 产品托管条款；当前仅推荐本地优先，不把适用条件等同于产品商业模式已决定
- **精确参考**：ACPHub 正确、可读的源地址；未取得不影响建议主线
- **真实账户**：用户具体 plan／组织／模型 entitlement、实际额度信号、独立配置／keychain 隔离、账户间上下文流转许可
- **技术实测**：锁定待实现版本；特别比对 Pi HEAD 与 release、Claude TypeScript SDK 生命周期、各网关流／工具保真与可安全切换边界

## 本轮回归与接续

本轮仅文档：检查相对链接／标题锚点、`git diff --check`、`git diff --cached --check`、提交范围与 Git 身份。没有产品测试可运行；不能将文档校验写成模型或 Harness 验证通过。

本次复核已归入[架构方案](../../docs/design/architecture.md)和 PRD。接下来先保留具体原型清单；收到实现／试验授权才改为执行任务。不要从本轮设计授权自行推进登录、收费实验或产品实现。本地 commit 仍不等于远端或长期持久托管。
