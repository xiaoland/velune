# 集中认证管理：实施与隔离验收

复核日期：2026-10-05。安装产品源码为 `31b394c`，manifest 为 clean、schema 3、`0.1 beta.1`；已安装至 `/Applications/Velune.app`。本记录说明实际交付与人工验收，不替代 [AI 服务设计](../../docs/design/ai-service.md)。模型身份及参数修正仍属于研究建议，未修改模型源码，见 [模型业务研究](model-business-research.md)。

## 实际边界

application 集中登记认证资源、协议及端点授权、generation 和内部 locator。提供商只保存 `authenticationId`，gateway 只依赖异步 `CredentialResolver`，AI provider 只消费瞬时认证。网关和 Mac Keychain helper 不再接受 Harness 来源 JSON；运行时来源 adapter 在 application 内装配。认证资源的纯配置操作由 local 与 portable 共用，保存时统一校验登记资源和目标，关闭本地运行时能力不能绕过约束。

Mac 增加独立原生“认证”设置页。API key 写入系统 Keychain；订阅和导入来源仍可委托原认证存储，集中管理其授权与生命周期，不复制 refresh credential、不删除原文件。集中管理不等于所有秘密已迁入 Velune 的存储。提供商页选择已登记资源，并采用其已授权协议和服务地址；Pi 仅显示在独立的来源信息中，不充当认证方式。

替换秘密先创建随机新 Keychain 项，配置提交成功后再返回可清理的精确 owned 引用。明确未提交的错误才允许删除本次新项；结果未知则保留。旧 Runner 退出失败返回已提交结果和清理暂缓说明。正在使用的资源不能删除，另一资源仍引用的旧项不清理。平台删除还限定固定 Keychain service 和应用生成的 account 格式；迁移前的外部项不自动删除。

schema 2→3 迁移只变换普通配置，将每个旧提供商来源／引用登记为独立资源，不按路径合并、不读取秘密或原 auth 文件。认证变更使相关导入预览失效，避免旧预览覆盖新认证。

## 验证证据

类型与静态检查、裁剪构建和 Mac 发行构建均退出 0：

```sh
cargo fmt --all --check
cargo check --locked
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo build --locked -p velune-bindings --no-default-features --target-dir target/portable-auth
bash scripts/build-macos.sh --build-only
```

Mac owner 还通过了新生成 UniFFI 模块、全部 Mac 源文件及 Keychain helper 的完整 Swift typecheck。发行 bundle 通过 deep／strict codesign 校验；安装时仅退出已确认的 Velune 进程，没有启动真实用户应用会话。

手动入口均使用临时 HOME、配置、来源和合成数据，不接入 CI、不新增自动化测试：

| 入口 | 实际结果 |
| --- | --- |
| [集中认证脚本](../../scripts/manual-authentication-management.py) | 分别加载无 local-runtime 库和已安装库，确认迁移、重开持久配置、未知资源／目标拒绝、使用中删除拒绝、generation 冲突、owned 清理与共享引用保护；非法迁移保留原文件；秘密 helper 调用与原来源读取均为 0。 |
| [原生 HTTP 脚本](../../scripts/manual-gateway-native.py) | 合成异步 resolver 下，9 个 Chat／Responses JSON、SSE、错误及业务终态边界通过；未知字段、历史、状态和安全 headers 保留；断开前／流中取消通过，Runner drop 为 0 ms。 |
| [安装包 Pi 脚本](../../scripts/manual-pi-native-loop.py) | 已配置运行时→选中提供商导入→路由→连接→创建会话→工具调用→续写→下一轮，共 3 个合成上游请求；实际固定 Pi 来源认证 adapter、推理历史与工具关联通过。原来源文件不变；跳过导入保留连接、替换断开旧连接、认证替换使旧预览失效。 |
| 同一安装包脚本的 helper 阶段 | 经 application 登记 API key 资源启动合成挂起 helper；客户端断开、约 30 秒解析期限及 application 关闭均终止其 Node 子进程；关闭不超过 3 秒。 |

首次 helper 验证把受管模型目录里的 `$VELUNE_GATEWAY_TOKEN` 占位符误作本地 token，未进入解析器。只修正了验收脚本：由临时 Node launcher 捕获本 fixture 注入的短期本地 token，未读取任何真实进程、密钥或配置。随后完整入口退出 0；安装产品未因此修改。原生 HTTP 脚本也改由 Cargo 输出选择匹配的依赖产物，避免不同 feature 构建的 rlib 混用。

本轮没有真实提供商调用、真实 Keychain 操作、登录或 UI 验收。Windows helper 的进程树生命周期仍未开放；自动 fail-over 继续 Disabled。模型 ID 与全局参数表单的业务错误已记录，不能将本次认证交付解释为它们已修复。
