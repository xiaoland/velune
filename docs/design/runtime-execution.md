# 运行时接入与执行

本页维护 application、agent-runtime、gateway 与平台之间的执行契约。职责装配见[架构](architecture.md)，产品行为见[Agent 运行时](../prd/agent-runtimes.md)，会话投影与接续见[会话与协作](conversations.md)。

## 运行时

运行时区分类型与实例，同一类型可保存多个实例。类型是带兼容版本区间的 versioned variant（[S34](../sources/runtime-collaboration.md#s34)、[S44](../sources/runtime-collaboration.md#s44)），family 只用于展示分组，不作为派发键；区间外的版本明确拒绝。各 variant、区间与原生控制方式见 [agent-runtime](../../packages/agent-runtime/README.md#版本与原生控制)。运行时 descriptor 声明协议、启动选项、会话选项与管理能力，平台据此展示，不按类型 ID 分支。

平台 app 不捆绑任何 Agent runtime。适配器从实例配置的外部安装解析 CLI 或 SDK；执行、历史、导入与来源认证不从应用资源或其他实例回退。应用只分发薄 adapter helpers 与只读历史 parser。

### 执行位置、HOME 与模型来源

三个选择相互独立：运行位置决定本机或 SSH，HOME 归属决定实例使用的用户家目录（继承、现有路径或 Velune 管理），模型来源决定本轮由 Harness 原生提供商还是 Velune 网关执行。application 解析实例配置并协调准备与释放，agent-runtime 封装原生协议与进程。

- **模型来源**：Velune 模型条目身份与运行时原生模型身份分开。原生选择附属于一个实例，不按相同 model ID 推断跨提供商等价；使用原生模型不需要 Velune 提供商、复制认证或启动网关。
- **单轮网关覆盖**：managed 轮次的网关接入属于执行环境，不写入持久配置。application 捕获本轮路由并准备 adapter 的原生参数，工作结束或失败时释放临时进程与网关访问。没有运行中的 managed 轮次时，Harness 的持久提供商配置保持原样可用。
- **HOME**：HOME 是 Harness 及其工具子进程看到的家目录，agentDir 是 Harness 的配置与状态目录，cwd 是会话工作目录。Velune 管理的 HOME 按实例稳定分配，配置重置或执行结束不删除其中的历史与认证；它提供环境归属，不承诺阻止工具访问其他文件。
- **SSH**：版本检查、模型发现、启动、恢复与历史操作都在执行机器完成；binary、agentDir、Node 与 cwd 按执行机器的路径解释。远端不启动常驻 Velune Host，也不把原生会话复制到本机。managed 轮次与协作 MCP 经远端 loopback 的反向转发访问本机服务，随执行资源释放；断连不自动重发。跨机器接续沿用来源 cwd，目标必须存在同一路径。SSH 连接认证归 OpenSSH，模型认证归 Harness 或网关。

### 运行时工作的生命周期

运行时进程与一轮的临时网关覆盖持续到该轮工作全部结束；原生子 Agent 与后台任务也算工作（[PRD](../prd/agent-runtimes.md)）。检测归 adapter，寿命策略归 application：

- agent-runtime 的 `NativeSession` 以 `turn_running()` 与 `work()` 区分轮次与轮次外工作。Codex 跟踪同一 app-server 中其他线程的状态与轮次，Claude Code 取未结束的任务；ACP、DSH 与 Pi 暂无轮次外工作。
- application 以“轮次运行或仍有工作”判定会话执行中。经网关的轮次在两者都结束时才结束进程；Codex 随后以短生命周期 app-server 把该进程运行过的线程用原生提供商 `thread/resume` 一次，使线程回到原生提供商，再释放 runner 与隧道。回收、退出、跟随来源、删除、归档与编辑都把这种会话视为执行中。
- 下一轮能复用进程时照常发送。需要另起进程（选择、提供商或会话变化；Claude Code 每轮都是）时，`restart_needed` 与 `ensure_work_ended` 以 `Error::OutstandingWork` 拒绝，不结束子 Agent；消息进入发送队列并标记 `FollowUp.waiting = SubAgents`，工作结束后自动发出。
- 停止父会话同时中断其子 Agent；`stop_sub_agents` 只停止子 Agent。退出时若有轮次或子 Agent 在 Velune 的进程中运行，平台先确认，再以 `shutdown_stopping_work` 结束它们。

### 通用 ACP

`agent-runtime::acp` 是独立的 ACP v1 客户端，连接用户安装的原生 ACP Agent 或社区 bridge；application 只装配执行位置、HOME、可执行文件、参数与显式环境。新增 Agent 不需要新的类型分支、私有历史解析器或版本正则；协议主版本由 `initialize` 协商，程序名称、版本与可选能力只作为数据。官方 Rust schema 定义 wire 类型，Velune 拥有会话投影与进程生命周期。

ACP 与专用 adapter 并存，`acp-v1` 不支持 Velune managed 网关。模型与其他会话选项使用 Agent 提供的 config ID、category 与 opaque value，以 Agent 返回的结果为准；查询模型不创建隐藏会话，模型切换不重启进程。list、load、resume、close、delete 按能力调用：load 回放原生历史，resume 不提供历史；没有 load 的 Agent 仍可连续多轮使用，但关闭后不保证重开，也不能作为跨 Harness 接续的源或目标。客户端只声明实际实现的能力，文件与终端回调不在范围内。

### 原生资源管理

agent-runtime 的 `management` 拥有经过版本验证的原生资源位置、格式与文件操作：用户指令、MCP 配置、skills 与原生子 Agent 定义。application 解析实例、用户或项目范围、执行机器与 HOME，并执行忙碌门禁。原生配置仍是权威，受管 HOME 中的文件同样如此，不形成 Velune 镜像；平台以资源库为唯一编辑入口（[PRD](../prd/resource-library.md)，实现见 [application](../../packages/application/README.md)）。

读取不因目录缺失而创建 HOME 或配置；编辑以预期 revision 避免覆盖已观察到的外部修改。共享 MCP 配置只公开与修改目标服务条目，列表、Debug 与日志不含环境变量或 header 值；各 Harness 的格式不统一转换，坏配置报告无效并拒绝覆盖。skill 是目录资源，按单文件读写并保留执行位，不跟随符号链接或特殊文件。修改在运行中拒绝，不自动重启会话；保存成功只表示原生配置已写入，何时加载由 Harness 决定。管理范围不等于祖先目录、插件或组织策略合并后的有效配置。

## 模型数据面

选择 Velune 模型的轮次走 `Harness → Velune 网关 → ai-provider → 上游`，选择原生模型的轮次由 Harness 沿用其原提供商。执行侧 Harness 只持有网关的本地访问凭据，上游授权在 ai-provider 的装配边界应用；application 私有保存提供商认证，网关只接收中立的异步解析接口。

| Harness | managed 轮次的接入 |
| --- | --- |
| Codex | 线程配置注入名为 `velune` 的 Responses provider，指向网关 |
| Claude Code | `ANTHROPIC_BASE_URL` 指向网关的 Messages 入口；Claude 订阅另经本机 Claude Code 执行后端，不复制 Claude token |
| Pi | SDK 追加独立生成的网关提供商，不覆盖原 `models.json`、认证或设置 |
| DeepSeek Harness | 执行 overlay 追加网关模型目录，不改原生默认选择 |

运行时 descriptor 的 `supportedProtocols` 是它发给网关的协议，`supportedProviderProtocols` 是经网关可到达的提供商协议；协议可到达不等于具体模型支持全部工具、输入模态或推理参数。路由策略、额度、fail-over 与协议转换见 [AI 服务设计](ai-service.md)。

**Pi 的模型身份**：Pi 的 managed 执行使用稳定的 `velune/auto` virtual model，由 Core 决定本轮的具体逻辑模型并返回其能力；网关执行同一次决定，不在请求到达时按可变的全局选择改投。Pi 按完整 `provider/api/model` 判断历史兼容性，因此物理模型 ID 使用非秘密路由绑定的稳定 SHA-256 身份，昵称、预算与 Node 路径不参与；界面与选择状态保留逻辑模型。已知上下文窗口与输出上限在扩展准入前交给虚拟模型；未知容量保持未知，不以强制填写作为通用执行门槛，需要该能力的扩展可明确拒绝。

应用内更新 API key 或完成来源登录会增加认证绑定 generation，使执行准备失效，下一次发送重新准备；认证刷新不改变 generation。外部在同一来源文件背后更换账户无法可靠辨识，应在应用中重新登录或更换认证来源。
