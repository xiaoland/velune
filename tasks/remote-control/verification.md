# 远程控制隔离验收

本页归属本任务的人工验收证据；不代替产品验收，也不声称已验证公网 NAT、中继运营或移动端生命周期。2026-10-17 开始，已用修复后的本轮冻结产物执行正式隔离验收，22 项观察全部通过；没有跳过场景。

## 判定边界

入口是显式运行的 `scripts/manual-remote-control.py`。使用实际 iroh endpoint、独立客户端身份、唯一 Application owner 和 UniFFI 业务接口；执行端与客户端都在本机隔离目录，连接经本机 QUIC。临时 HOME、应用根目录、原生 Harness 记录、设备密钥和资源均由脚本创建，不读取真实配置、认证或会话，不调用模型、SSH 或公共 relay。

合成 ACP peer 复用既有人工验收 fixture，覆盖真实 application 业务生命周期，不用网络回声代替会话、审批、队列和协作。验收过程中不新增或运行自动化测试，不接入 CI。

| 约束 | 足以支持结论的观察 | 当前证据 |
| --- | --- | --- |
| 业务唯一 owner | 本地配置变化在远控可见；远控资源、会话和变更查询由同一 application 实例执行 | 通过 |
| 完整业务类别 | 远端保存和读取提供商、显式查看合成 key、资源读写、会话收发、审批、工作区 diff | 通过 |
| 信任与撤销 | 未配对身份被拒；一次性授权不可复用；revoke 后原已连接客户端不能继续读写 | 通过 |
| 控制端离线继续 | 全部客户端断开后，合成 peer 完成 turn，队列实际发出下一条消息；协作子会话仍推进 | 通过 |
| 另一端接续 | 第二个已配对端取得更新投影与真实待决审批，回答后 turn 收束 | 通过 |
| 请求副作用 | 已接纳请求丢回执后按 request ID 查询结果；相同 ID 不重复执行；无记录返回未知 | 通过 |
| 审批竞争 | 两端针对同一审批回答，只有第一个成功，peer 只收到一次回答 | 通过 |
| 写入冲突 | 使用旧配置／资源／编辑 revision 的第二次写入被拒，当前内容保持第一次结果 | 通过 |
| 多视图隔离 | A、B 选择不同 runtime 与归档视图，互不改变对方的同名视图字段；业务变更仍共享 | 通过 |
| 路径与秘密 | 控制端路径被拒，显式内容上传后删除控制端文件仍可发送；合成 key／ACP 环境秘密不自动进入配置摘要或诊断，普通 runtime 编辑不丢失隐藏 launch | 通过 |
| 大内容预算 | 17 MiB 合成 PNG 经上传、真实 ACP 输入和显式内容读取保持 SHA-256 一致；20 MiB + 1 字节被拒且不产生暂存文件 | 通过 |
| 同对象并发 | 一个远程 VeluneApplication 的真实 ACP 初始化等待 gate 时，同一对象的 request 状态查询在 gate 释放前返回；不以两个对象或纯传输多流代替证据 | 通过 |
| 连接路径 | 记录实际 connection status；loopback 且关闭 relay 的结果只支持本机直连。中继／跨 NAT 另立证据 | 通过 |

## 执行记录

构建基线为 `6b1a543` 加本任务工作区改动；传输依赖固定 iroh `1.3.0`。环境为 rustc `1.99.0 (b940084d7 2026-09-28)`、Python `3.12.10`。入口及生成 ACP fixture 已通过 Python 语法检查，`--help` 可用。

主执行者统一构建并冻结动态库和 Python bindings 后，正式入口执行到 generic ACP 会话的 `working_changes` 时失败：`Invalid / 此运行时没有原生配置目录`。使用 managed HOME、合法 generic ACP、无原生 agentDir 的合成会话可以创建，但变更查询不应把 Harness 原生配置目录当作工作区事实。application owner 已将 `conversation_directory` 的执行位置判别从要求原生 agentDir 的 `execution_context` 改为 `runtime_location`，共用此入口的 working_changes／working_branch／file_diff 同时修复；静态检查通过，新冻结动态库正式全量执行通过。脚本保留 managed HOME、无 agentDir 的原判据，已同时读取真实 git 变更与具名 diff，并拒绝 `../outside` 路径。

为继续独立验证其它约束，诊断执行仅在内存中省略已知失败的 working_changes／file_diff 段，输出明确标为 `DIAGNOSTIC_PARTIAL`；源脚本没有隐藏失败。该诊断阶段只用于继续发现独立缺口。修复后正式入口原样全量执行，输出 `acceptance=PASSED`，22 项观察全部通过，包括真实审批竞争、raw permission ID 复用隔离、旧消息编辑以 Conflict 拒绝且无副作用、二进制 skill 与模板存储、同对象并发、断线工作推进和撤销。

正式入口命令：

```sh
python3 scripts/manual-remote-control.py \
  --library "$PWD/target/remote-control-bindings/libvelune_bindings.dylib" \
  --bindings "$PWD/target/remote-control-bindings" \
  --resources "$PWD/target/runtime-support"
```

冻结产物 SHA-256：动态库 `b57fad02bc813288a6b020c80e48da7f175000ed7cb035602c2679377f09b29b`，Python bindings `195689ad23e46ca0a8d899115c162c723e90b9c4cae15ebeffbc8442d2f11770`。

17 MiB 合成 PNG 的上传、实际 ACP 图片输入与显式内容读取均为 17,825,792 字节，SHA-256 为 `d5a0be16a523c3a3c5b0c7d5c09519e0272f37428c3dc433909a5bb20d0524cd`；ACP 输入 MIME 为 `image/png`，20 MiB + 1 字节上传被拒且未新增暂存文件。PNG 仅满足现有入口的类型签名约束，未声称经过图片解码器或模型视觉能力验证。

正式执行连接的实际状态为 Direct、selected=true，endpoint 仅绑定 `127.0.0.1` 且 relay_urls 为空；采样 RTT 817 µs，发送 13,226 字节、接收 26,248 字节、丢包计数 0。这些是该次采样的本机路径观察，不是公网性能结论。

人工脚本修正了三个不符合当前契约的判据：新空 HOME 尚无 gateway；普通附件在投影中是具名 File 块，不能要求路径仍在 Text；`AttachmentContent.media_type` 允许 None，改以实际 ACP MIME 与内容字节判定。另修正合成 Alpha 的观察文件中 tool content 是数组的读取方式。以上均为验收 fixture／oracle 修正，没有为使验收通过而改变产品契约。

## 已复用的独立证据

主执行者使用同一冻结 debug 产物、临时伪 bundle、空 Resources 和合成 HOME 执行 `scripts/manual-provider-configuration.py` 并报告通过，覆盖提供商保存／编辑、Key 显式读取修改和重开、秘密不入摘要日志、模型模板、无效配置不变、私有文件 0600 及 hard-cutoff。该脚本此前仍使用旧 schema 9／future 10，已对齐当前 10／future 11；Clear 后按既有 `read_api_key` 契约断言空字符串且 configured=false，未把旧脚本的“应抛错”判据变成产品改动。

## 未覆盖

视图行为实际覆盖 runtime 选择与归档开关，没有单独遍历全部分页／临时过滤组合；消息编辑验证的是陈旧 revision 在派发前以 Conflict 拒绝，不声称 generic ACP 支持成功的原生历史重写。请求恢复通过故意丢弃 submit 回执并关闭控制连接来验证，没有注入物理网络丢包；不承诺执行端崩溃后的 exactly-once。

未验证跨公网 NAT、严格代理、移动后台／休眠与运营级 relay 可用性；本机 direct 证据不能替代这些现场验收。传输 owner 的显式公共开发 relay／RelayOnly 探测单独归主执行者集成，不将它混入本页的 Application／UniFFI 本机证据。没有调用真实模型、认证、SSH 或真实会话，没有新增或运行自动化测试。
