# 用量与性能分析

用户于2026-10-07授权增加“分析”，参考Magpie／ccusage／sub2api，目标是理解 token 消耗与性能。长期意图归 [PRD](../../docs/prd/index.md)，跨 unit 计量契约归 [AI 服务设计](../../docs/design/ai-service.md#用量与性能分析契约)。首版统计新增的 Velune 网关请求，不导入运行时历史、不估算费用或订阅余额；已向用户说明此实施范围，历史范围异步问题尚未回复。

root 负责协议计量、网关关联、application 存储／聚合、具名绑定、文档与最终安装。analytics_boundary_advisor 提供数据权威与指标口径判断；ai_service_audit 完成初始 Rust 切片后冻结，由 root 接管并修正完整人口聚合、整数校验、异步存储失败隔离与原生协议事实。session_management 是 Mac 稳定 owner，贯通实际 Store／Transport／UniFFI 和原生分析窗口。只用临时 HOME、合成 HTTP 和合成数据，不读取真实配置、秘密或会话，不调用真实模型，不新增自动化测试。

## 当前实现

ai-provider 在原生 JSON／SSE 中提取独立未知／已报告 token 数和业务终态；SSE 使用既有 eventsource-stream 分帧，累计 usage 覆盖而非相加。gateway 在协议转换前记录实际提供商／模型、时间和请求结果，注入非阻塞中立接收器。同协议正文不改写，转换占位不能进入计量。application 独立维护 analytics.sqlite，后台有界队列写入，异常明确告警而不阻止应用配置和执行。查询遍历全部匹配记录，仅请求明细页限500；总量采用输入和输出共同已报告的 cohort，速度采用输出已报告且实际终态耗时有效的同一 cohort。Messages 普通输入、缓存读取和创建分别保留；缺少缓存组成时总输入未知。

Mac 使用独立原生分析窗口，入口在主工具栏与显示菜单，未占设置 Tabs。今天／7天／30天使用本地 Calendar 日边界，概览后逐级进入每日趋势、提供商／模型拆分与请求详情。采用系统 Charts／Table，未知显示破折号，报告零保留零；来源范围和有效速率口径可查看。分析查询 generation 隔离，不共用会话 loading 状态。

## 隔离证据

2026-10-07：fmt、locked cargo check、workspace all-targets/all-features Clippy warnings-as-errors 通过；无 local-runtime 的 application 静态构建已通过。Mac fresh UniFFI 绑定、全产品 Swift warnings-as-errors 构建通过。

`manual-gateway-analytics.py` 用实际 loopback HTTP、Runner 和 application AnalyticsStore 走通14例：三协议 JSON／分块 SSE、累计 usage、缺失计量、断流、429、取消、转换占位隔离、Messages 部分输入以及真实零。关闭后重新打开实际 SQLite，原生响应字节保持一致。合成数据为14请求、输入112、输出50、配对总量159（11请求覆盖）、9请求可计算速度。最初手工预期8遗漏普通输入未知但输出有效的请求，依据原始事实修正为9，没有修改产品算法迎合预期。

`manual-analytics-storage.py` 使用实际 UniFFI 查询验证全人口统计与明细分页独立、成功 cohort 速度、重开持久化；100004条记录没有100000条人口截断，负数计量明确失败，不可用统计库不阻止应用打开并返回可见告警。该脚本按需运行，不接入 CI。

Mac `manual-analytics-ui.py` 使用上述实际采集 SQLite，贯通真实 Store／Transport／UniFFI，验证筛选、覆盖率、未知与零、部分输入、迟到请求隔离、Problems、会话状态独立和23小时夏令时日。原生 GUI 视觉和真实提供商由用户验收，不以这些检查冒充。

剩余：owner 最终冻结后干净 release 构建并安装 /Applications/Velune.app，记录源码提交与安装证据。当前版本保持0.1 beta.1，不远端发布。
