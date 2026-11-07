# 官方 API 提供商模板与接入产品边界

2026-11-02。用户希望补齐 OpenAI Platform、Anthropic API，并要求明确 Moonshot API 与 Kimi Code 的差异，避免随提供商增长不断追加特殊分支。本轮按现有提供商产品约定完成必要实现与隔离验证；不提交、不重装日常应用、不使用真实凭据或计费请求。保留工作区其他任务的修改。

当前结果：官方 API 模板与组织账单改造已实现并完成合成验证；当前交付和平台限制见[组织账单改造实施](#组织账单改造实施2026-11-02)。用户随后修订迁移约定，本次 schema 11 升级前须完成显式手动迁移，当前尚未执行。

## 设计判断

- 品牌不决定接入产品，接入产品也不等于协议。已有 `Protocol`、`ExecutionBackend`、`RequestProfile`、认证来源和额度来源分别承担职责，继续保留；模板仅是初始快照，不增加持久化品牌或账户实体。
- 新增 OpenAI Platform（Responses、`https://api.openai.com/v1`）和 Anthropic API（Messages、`https://api.anthropic.com`），均为 HTTP、Native、API key、默认不读取账户额度。协议与地址可编辑，模型继续手填或从已有模型目录／模板选取。
- 现有 API 构造函数把所有官方 API 都等同于 ChatCompletions＋账户额度，需改为显式接收协议与额度来源。
- Mac 按 profile／认证硬编码订阅名称，Windows 从模板匹配订阅。将匹配和元数据查找统一到中立目录，供两端使用；保存校验、认证解析与请求执行仍由原责任方执行。
- Moonshot API 与 Kimi Coding Plan 保持独立模板和配置；明确不同 Key、地址与计费，协议不能作为订阅身份判据。

## 依据

- [提供商模板与产品约定](../../docs/prd/ai-providers.md#提供商模板与来源)、[中立模板目录](../../packages/application-api/src/provider_template.rs)、[组合校验](../../packages/application/src/config.rs)。
- [OpenAI 官方快速开始](https://developers.openai.com/api/docs/quickstart)：Responses URL 与 Bearer key。
- [Anthropic API 概览](https://platform.claude.com/docs/en/api/overview)：Messages URL、API key 认证和版本头；现有原生路径已支持 x-api-key。
- [Kimi 官方 FAQ](https://www.kimi.com/en/help/kimi-code/faq)：Coding 与开放平台的 Base URL、Key 来源和计费不同。

## 验证与进度

实现、共享绑定行为验收与两个平台构建完成；Windows 深色主题及正常退出检查仍未完成。判断条件：两项模板可由共享绑定获取并作为可编辑 API 配置保存；API 不被认成订阅；Moonshot 与 Kimi 的组合不混淆；Rust 检查、Mac 构建、Windows 所需检查及文档链接检查得到明确结果。用合成数据和回环上游确认路径与认证头，不把隔离验证当作真实账户联调。

已取得证据：

- Rust release 动态库构建通过；隔离 target 的 `cargo check --locked -p velune-bindings --no-default-features` 与严格 clippy（all-targets、`-D warnings`）通过。
- 通过当前 release 库生成的 Python 绑定验证全部 10 个模板、订阅匹配、CLI 路径变化、Kimi／Moonshot 的 API key 区分、两项官方 API 保存与重开，以及修改协议／地址后再次保存。日志 `/tmp/velune-provider-behavior.log`，合成数据目录 `/var/folders/rk/8_krr0y14p9g5plk8n77lgyr0000gn/T/velune-api-providers-lz_wevbx`。
- 已安装 Pi 1.4.0＋临时 HOME＋回环上游，经真实 application → gateway → native HTTP 完成两个模板的流式对话。OpenAI 到 `/v1/responses` 使用 Bearer key、不携带 Codex 账户头；Anthropic 到 `/v1/messages` 使用 x-api-key 并携带版本头，两轮结果均 completed。日志 `/tmp/velune-provider-http.log`，合成数据目录 `/var/folders/rk/8_krr0y14p9g5plk8n77lgyr0000gn/T/velune-provider-http-32btln_v`。
- 修改的三个 Rust 文件独立格式检查、5 份文档相对链接／标题锚点与 `git diff --check` 通过。
- 临时探针在 `/tmp/velune-provider-*.py`，不进入仓库。首次 Python 导入方式、PowerShell stderr 重定向出现的临时验证脚本问题已纠正，不属于产品缺陷。

平台验证：

- SVC Mac 构建通过，隔离预览副本重签及严格签名校验通过；执行 `6b3027a4-7a55-4837-9df9-7b2e10caff70`，2026-11-02 11:18:15 UTC 完成。该调用跟随已在进行的同一构建；核对生成 Swift 绑定已包含新函数、相关 Swift 编译通过。隔离副本以 `local.velune.visual-preview` 启动，通过原生 AX 实际打开添加目录及两项 API 表单，确认默认协议／地址、可编辑字段和 API key 认证，未显示订阅锁定或不支持的额度查询；预览进程已正常退出。
- Windows 以当前工作区快照在 `F:\CODING\.worktrees\api-provider-templates\velune` 执行原 `scripts/build-windows.ps1`，Rust release、生成 C#、warnings-as-errors publish 及资源打包通过。日常仓库保持原 revision，未重装日常应用。Windows 界面验证副本仅增加测试环境变量选择 RequestedTheme，原任务源码不含此入口；该独立 C# publish 也通过。

## 交付边界与保留事项

- Mac 原生表单检查、核心保存／重开与两个真实 Pi 流式合成请求通过。未使用真实 API key、读取真实账户或请求公网模型；OpenAI／Anthropic 真实账户行为仍由用户验收。
- Windows 浅色设置页、共享模板目录和 OpenAI 编辑表单已通过 RDP 原生键盘检查，字段可编辑、默认值正确，没有订阅固定连接或额度开关。运行超过 3 分钟，合成 TEMP 下没有 crash.log。鼠标操作返回 `noWindowsAvailable`，键盘替代完成上述检查；随后 RDP 的前景桌面发生变化，无法继续可靠地定位测试窗口。深色主题、Windows Anthropic 表单和正常退出码 0 未取得证据，不能称为平台界面全验收通过。已停止仅本次路径的验证进程并清理计划任务；没有操作日常 Velune 进程或安装。
- 余项：在可稳定操作的 Windows 原生自动化会话完成深／浅色提供商表单与正常退出复验。Windows 源码构建已通过，这项证据缺口不等同于已发现产品缺陷。
- 不提交，不重装。Mac 使用新版本需用户在 Velune 外执行 `svc run reinstall-macos`。

## 账户额度查询调研（2026-11-02）

用户追加希望评估 OpenAI／Anthropic 的账户额度查询并查看 Mermaid 架构。本轮交付为官方接口调研与建议，未新增采集代码或访问真实账户。完整[调研与架构图](account-observability.md)保留接口、权限、事实边界和最小演进。

- 两家均可查询组织用量与费用，需要组织级查询权限；OpenAI 还可读官方月度支出限额。Anthropic 新推荐 Analytics API 当前 beta，普通 workspace 模型 Key 不适用，个人账户不能使用 Admin API。
- 本轮未找到公开官方充值余额接口；账单支出、预算限额、速率限制与余额必须分开表达。
- 建议提供商下可选配置查询认证与统计范围，由 application 独立采集并展示有类型的账单事实；Admin 凭据不进入模型调用，报表暂不用于网关准入。此为建议，尚未批准或实施；此前实现与 Windows 验证余项保持原状态。

## 组织账单改造实施（2026-11-02）

用户已确认“好的，开始改造”，授权实现上一轮方案；不提交、不使用真实查询凭据、不重装日常应用。

执行顺序：

1. 增加共享的账单配置、范围、编辑命令与有类型的报表；查询凭据与模型认证分离，统一现有 AK/SK 的秘密表示。
2. 实现 application 官方适配、分页、有界缓存、刷新与失败／过期状态，不连接网关准入。
3. 在 Mac 和 Windows 提供商下提供可选的配置与只读报表入口。
4. 用合成上游验证不同 scope、分页、权限失败、部分结果、换凭据与删提供商，不泄露 Admin Key；完成构建、原生界面与文档检查。
5. 将已实现职责和 Mermaid 图整合至权威文档，保留真实账户和平台验证的实际边界。

共享契约、私有查询凭据、官方采集与缓存、Mac／Windows 独立面板均已实现。授权范围内的核心与平台验证已完成，未提交、未重装；完整 Mac 入口与真实账户的证据边界见下文。

- 结果契约与 Mermaid 已整合至 [AI 服务设计](../../docs/product-tdd/ai-service.md#组织用量与账单)，产品语义归 [PRD](../../docs/prd/ai-providers.md#组织用量与账单)。查询凭据不进入模型调用，报表不进入准入。
- 配置 schema 从 10 升至 11：查询秘密统一为有类型的 API key／AK-SK。此前按旧约定交付为“旧普通配置重置、不迁移”；用户随后要求必须显式手动迁移，该交付结论已修订。本次版本切换前须完成 10→11 的手动迁移与核验，当前尚未执行；原 Harness 会话与认证不在重置范围内。
- Rust 核心／绑定 all-targets 检查与严格 clippy 通过；纯 remote 绑定严格 clippy 通过。4 项不变量检查通过：官方金额单位与缓存 token 语义、过滤／分页、失败／重复游标／重定向不发布部分总额、缓存权限／scope／月份隔离，以及取消后同身份重建时旧结果拒收。日志 `/tmp/velune-billing-tests-final.log`、`/tmp/velune-billing-clippy-final.log` 与 `/tmp/velune-billing-remote.log`。
- 真实 release 绑定＋临时 HOME＋本机拒绝 CONNECT 的合成代理验证保存 rollback、模型／查询认证分离、重开、换模型 Key 失效、切换非官方目标清除、关闭／删除查询秘密；没有连接公网账单服务。日志 `/tmp/velune-billing-behavior.log`，最终数据 `/var/folders/rk/8_krr0y14p9g5plk8n77lgyr0000gn/T/velune-billing-behavior-xs5ijyr7`。
- 最终 SVC Mac 构建通过，执行 `3a292ac2-bcfe-49c0-b32d-c80f06cef42a`；Windows 隔离源码快照执行原 build-windows.ps1 成功，退出码 0，日志 `F:\CODING\.worktrees\velune-billing-final.{stdout,stderr}.log`。Windows 日常仓库和安装未改动。
- Mac 原生面板在独立 SwiftUI harness 中使用生产 `ProviderBilling.swift` 和生成绑定、合成状态观察：OpenAI 范围／费用／月度限额分开显示，确认关联控制保存按钮；Anthropic Workspace、官方刷新时间、beta 延迟提示和速率披露入口可见。时间统一改为显式 UTC。此为面板渲染证据，不替代完整应用入口验收；harness 的 DeviceStore 与披露装饰为测试替身，未查询真实服务。
- 完整 Mac 隔离副本启动后主线程持续更新 Composer，AX 连续超时；采样 `/tmp/velune-billing-ui-sample.txt` 指向现有 Composer SwiftUI 图更新，该实例已停止，没有修改其他任务的 Composer。独立面板正常渲染；完整入口的原生验收仍有此限制。
- Windows UI 验证副本仅增加主题选择和启动设置页，源码本体无测试入口。通过原生 UIAutomation 在浅、深色中各从提供商列表进入 OpenAI 与 Anthropic 编辑器及账单面板，展开查询配置并检查组织／范围／独立 Key 字段；缺少凭据时保存禁用，每个面板停留超过 32 秒触发轮询，未保存草稿未被重置。两主题均正常退出码 0，合成 TEMP 无 crash.log。证据 `F:\CODING\.worktrees\velune-billing-ui-check\{Light,Dark}\` 的 trace、typed UI tree、passed 与 exit；浅色 21:07:46、深色 21:09:39 完成。最初启动撞上 resources 重建和跨任务找窗口失败，已用最终产物、单任务连续流程复验通过；不将这些临时验证问题视为产品缺陷。

交付检查：PRD、AI 服务设计（含 Mermaid）、application／bindings／Mac 说明和 Windows PARITY 已同步；8 份相关文档的相对链接／锚点、相关 Rust 文件格式与 git diff --check 通过。Windows 原生脚本仅保留任务证据，已移除本次计划任务；隔离验证进程已退出。

证据边界：Windows 验证了实际生产配置与未配置查询凭据的入口／必填／轮询生命周期；真实报表采集由回环 HTTP 不变量检查覆盖，Windows 未联调真实账户。Mac 报表渲染由生产 View＋合成状态验证，完整应用入口需在 Composer 恢复响应后复验。真实组织认证与供应商规则未验收。

## 仓库迁移约定修订（2026-11-02）

用户明确要求：允许 hard-reset／hard-cutoff，但需要手动迁移；可手工完成或编写并显式执行一次性脚本，禁止建立自动迁移机制。仓库入口、开发说明、相关 PRD／技术说明与来源记录已同步，历史决定保留并注明后续修订。

本轮为约定与文档更新。现有加载器仍会重置未迁移的旧普通配置；真实 schema 10→11 的显式迁移及结果核验保留为升级前待办，尚未操作日常数据或重装应用。11月4日[工作区收尾](../workspace-delivery-and-network/packet.md)已补齐 `scripts/migrations/00-config-schema-11.py`：迁移 runner 首先转换查询 AK/SK 类型，保留认证与关联修订；预览、锁、备份、幂等、拒绝未知结构以及实际 bindings 重开均以合成数据验证通过。

验证：9 份相关文档的相对链接、标题锚点与 `git diff --check` 通过。
