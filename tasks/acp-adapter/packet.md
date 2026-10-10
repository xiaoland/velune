# 通用 ACP Adapter

## 目标与授权

2026-10-10 用户授权继续实现 Rust ACP Adapter，并参考 `lexoliu/acpsub`（用户随后更正原 acphub 拼写）。沿用上一轮 packages 范围：不修改或构建 App，界面需求归 PRD 并明确待实现；不读取真实凭据或会话，不调用真实模型，不 push。允许本地任务提交。

用户随后澄清目标是广泛支持 Harness 且避免逐个适配，ACP 是实现手段。当前工程选择为：原生 ACP Agent 或用户安装的社区 bridge 均通过通用启动命令和参数接入，不为每种 Harness 新增分支；自动安装／下载目录不在本轮范围。

通用接入应复用现有会话契约、本机／SSH 传输与 HOME 管理，通过 ACP 能力协商驱动创建、发送、流式投影、权限答复、取消与可用的历史恢复。不把特定 Harness 的私有 provider 注入或历史格式当作 ACP 标准。

## 已完成的实现

- `agent-runtime::acp::{Config, Client}` 使用官方 `agent-client-protocol-schema =1.11.0` 的 stable v1 类型。复用 stdio／SSH，按 initialize 协商，不按 Harness 名称或 CLI 版本白名单接入。
- 实现 new、prompt、消息／推理／工具投影、权限答复、cancel，以及按能力支持的 list、load、resume、close、delete。未知可选配置不阻断默认运行；模型与完整 select／boolean 配置保留 Agent 原始身份及有效返回值。
- application／bindings 提供 typed 启动配置、Agent 信息、认证方法描述、会话配置与 `RuntimeDefault`。无模型／无 list／无 load 的 Agent 可 live 多轮使用；无 load 不伪造历史恢复。普通查询和模型切换保留活跃进程。
- SSH 和 HOME 复用已有执行契约，ACP 不要求 Node、agentDir 或辅助资源目录。明确原生 ID＋cwd 可加载无 list 的会话；关联只补真实 cwd 元数据，不保存消息副本。持久跨 Harness 接续要求两端可重读原生历史，能力不足时保留来源并在创建目标前拒绝。
- 正常显式 shutdown 对支持 close 的空闲会话先等待 `session/close`，再发 EOF 并有界结束进程；失败仍清理并返回错误，Drop 不执行阻塞协议调用。
- 配置控制请求结果不确定时停止发送，不自动重试；已有消息保留。无原生 message ID 的展示身份与权限请求的进程身份分开，同一历史回放保持稳定；显式原生身份可通过身份确认替换占位。

长期语义已归位 [PRD](../../docs/prd/index.md)、[架构](../../docs/design/architecture.md#通用-acp-接入)、[开发说明](../../docs/development.md#通用-acp-agent) 及三个 unit README。用户澄清的广泛接入目标决定了本次采用通用启动配置与协议类型，而不是增加更多 Harness 分支。

## 验证与证据

静态与构建：`cargo fmt --all --check`、`cargo check --locked --workspace --all-targets --all-features`、全 workspace／targets／features 的 clippy `-D warnings`、bindings 无默认 features 的 check／clippy，以及 bindings 动态库构建均通过。按仓库规则未新增或运行自动化测试。

- [协议与参考判断](protocol-evidence.md)：`lexoliu/acpsub` commit `259db0d08f43941d3742d3a4710fd89f0adc603b` 与官方 schema commit `af9d8f5d3852f8c3c774b81f7366e6707e0fd5e1`，检查日期 2026-10-10；采用官方已发布 schema 1.11.0，不引入 acpsub daemon 或历史副本。
- [真实 DSH peer](native-peer-evidence.md)：通过新 Rust Client 对真实 DSH ACP 执行 new、配置、prompt、close、list、resume。上游为本机合成 Messages 服务，实际模型流进入 Rust snapshot；正常 shutdown 后新 Client 仍能列出原生会话，未调用真实模型。
- [真实 WSL SSH](ssh-evidence.md)：通过 UniFFI 与合成 ACP peer 验证字面 argv、managed HOME、空 resources／无 Node、配置、权限、取消、历史重开和正常／断连清理。自建远端目录与进程已清理。
- `scripts/manual-acp-adapter.py`：显式手动运行通过；覆盖无隐藏 new、无 load 多轮保持同进程、未知 option、live→load→load 稳定 ID、模型／布尔配置、权限无效与陈旧答复、取消、EOF 保留部分消息并禁止续发、有 load 无 list 的显式重开、能力不足 handoff 保留来源、ACP ↔ Claude 合成 peer 接续与重开，以及无 list 但可 delete 的单目标关联删除。

手动入口使用本轮生成的 Python UniFFI bindings 和当前 Rust 动态库：

```sh
python3 scripts/manual-acp-adapter.py \
  --library "$PWD/target/debug/libvelune_bindings.dylib" \
  --bindings "$PWD/target/acp-adapter-bindings" \
  --resources "$PWD/target/runtime-provider-ssh-resources"
```

该命令为隔离验收，不接入自动测试或 CI。资源参数只用于同时验收已有 Claude 专用 adapter；通用 ACP 自身用空资源目录。

## 明确保留的边界

App／前端未修改、未构建、未安装；启动配置、默认模型／会话选项及能力呈现明确待实现。未调用真实模型或执行真实登录，未逐个认证所有 ACP Harness。通用 ACP 无标准 gateway provider 注入，仍由专用 adapter 提供；不实现文件／终端客户端回调、自动登录、自动安装、MCP 服务或 daemon。SSH 需要 POSIX／Python 3，HOME 不是安全沙箱。

补充 shutdown 的手动 peer 验收已通过：成功 close 后收到 EOF，业务拒绝与无效响应返回错误但清理进程，无能力不发 close，Drop 不发 RPC，忙碌关闭终止进程，重复 shutdown 不重复调用。

实现与限定验收已完成；本轮本地提交包含代码、手动验收入口与文档，不 push。其它会话的 Mac 与滚动脚本工作不纳入本任务。
