# 待决事项与已知缺口

2026-10-29 起。架构重构阶段 6 合并并删除已完成的任务包时，把其中仍未解决的用户决定、未授权的候选与未验证的能力集中到这里；原任务包的过程与证据见 git 历史（`tasks/<名称>`）。解决一项后删除该项，并把结论写入对应的 PRD、设计文档或 README；本页清空后删除本任务包。

## 待用户决定

- **项目许可**：仓库没有项目许可。协议转换链接的 New API RelayKit 为 AGPLv3，且其 NOTICE 要求署名与界面链接（见 [ai-provider README](../../packages/ai-provider/README.md)）；Android 界面参考的开源客户端也有各自许可。若以开源方式发布需先选择许可，与依赖相容的常见选择为 Apache-2.0、MIT 或双许可；选择宽松许可时须单独评估 RelayKit 的 AGPL 义务。选定前不宣称 Velune 是开源软件。（原 `new-api-reuse`、`android-ux`）
- **配置跨端同步**：2026-10-19 用户决定另开任务实施，当前只保留扩展性（远控请求按操作分类型，可增量新增同步请求）。候选方案：完整端之间按提供商同步网关、提供商、模型与资源模板，带“逻辑时钟＋设备 ID”修订号与墓碑；设备绑定的认证、运行时实例、资源应用记录、界面偏好与会话整理不同步；同步复用远控授权、走 remote 协议的新增请求。待决：同步范围是否正确、界面偏好是否同步；API key 与资源模板中的秘密是否随同步传输（建议是）；凡已配对完整端自动同步，还是逐台开启（建议逐台开启并要求对方有控制授权）；冲突采用“后写者胜并保留被覆盖版本”还是逐次询问。实施时注意 `application::new_record_key` 由纳秒时间加进程内序号生成，跨设备应改为带设备限定或随机的键。（原 `remote-control-mvp`）
- **远控中继部署**：Mac 经 sing-box TUN 出网、手机为对称 NAT 时无法打洞，始终经公共 iroh relay，且 relay 按代理出口延迟选择，跨网打开会话很慢。代码无法单独消除：可让代理对 Velune 直连，或自建就近 iroh relay（核心已支持 `RelayConfig::Custom`，应用固定公共 relay）。属部署决定。（原 `remote-control-mvp`）
- **远程配置写入冲突后的恢复**：远程视图的期望 revision 只在取得远程应用对象和自身成功修改时推进；执行端配置被其他视图改过后，该远程视图的配置写入会一直冲突，直到重新取得远程应用对象或执行端重启。可选修正：远程视图重读配置时把读到的 revision 作为新的编辑基线。（[架构重构](../architecture-refactor/packet.md)）
- **界面词汇**：面板／工具窗口／区／块／弹出层的统一叫法尚未确定，未落入文档。（原 `composer-turn-settings`）
- **Pi 子 Agent 来源**：Pi 1.0.2 没有内置子 Agent，随包示例扩展不支持技能；用户提到的“官方 subagents 插件支持技能”与之不符，待用户确认来源。（原 `resource-subagent-fields`）

## 未授权的候选

- Claude Code 子 Agent 不在会话列表时，标题回退为工具名“Agent”。（原 `project-folders`）
- 新建会话没有“在项目中新建”入口；主文件夹目前只用于工作目录建议。（原 `project-folders`）
- 其他 Harness 会话列表的增量读取需要各自来源契约，按实际规模再定。（原 `transcript-data-path`）
- 已载入的超大工具输出仍整段排版；实际出现卡顿时再考虑截断显示（属于产品行为）。（原 `transcript-data-path`）
- 网关运行期冷却／探测状态没有 bindings 读取入口，界面不显示“冷却中”或逐次切换原因（切换记录在“分析”窗口）。（原 `model-routing`）
- OpenAR（工作名）：2026-10-11 用户不接受“Rust 管理按实例 Node worker”的路线并提出 Rust 生态的独立 agent runtime 库。建议从 `packages/agent-runtime` 提炼公共边界（会话与 turn 生命周期、类型化事件、宿主请求与回复、取消关闭、能力声明），第一切片以同一宿主接口运行任意 ACP 命令与 Codex 原生；Pi 可由 Rust 直接驱动官方 `--mode rpc`，以薄原生 extension 注入网关与协作 MCP，替换 `pi_rpc.mjs`。建议待用户采用，未开始迁移。（原 `oar-reuse`）

## 未验证的能力

- **Windows 界面**：中文输入法、双击与右键等真实指针操作、长转录的虚拟化与流式更新性能、Windows 11 Mica、标志在任务栏与标题栏的观感需真机验收；用户从 RDP 会话启动的三次崩溃没有 `crash.log`，原因未查明。键盘操作的命令面板、速查表与审批卡片尚未在浅色与深色下各打开一次核对。（原 `windows-winui`、`full-keyboard`）
- **远控网络**：WAN→LAN→WAN 漫游、真实 NAT／UDP 故障回落、蜂窝默认且 Wi-Fi LAN 保留、长时后台、IPv6-only mDNS 与公共设施 SLA 没有证据。非空 mDNS 范围仍保留上游 wildcard 接收／发送回落，不是严格的网卡流量白名单。（原 `android-minimal`）
- **xURL**：应用 URI 查询只支持标题 q 与 limit；官方 `-d` 使用独立原生认证与执行，不继承 Velune 审批、模型路由或取消。真实账户、外部 SSH 运行时及公证未验收。（原 `xurl-support`）
- **真实服务**：多数功能只以合成上游与隔离 HOME 验收，真实提供商、真实订阅登录与额度接口、各平台 GUI 体验由用户验收。
