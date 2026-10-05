# Codex 与 DeepSeek Harness 接入

## 授权与目标

2026-10-06 用户授权支持 Codex app-server 与 deepseek-harness，直接依赖 huihua 发行包读取会话，不复制解析器源码，打包许可证与声明。不同 breaking-change 版本属于独立运行时类型，而非同一适配器的错误提示；familyId 归类、typeId 选择适配器、versionRegex 校验实际发行版本。用户次日执行 GUI 与真实服务验收。稳定产品意图归 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)。

## 实施与边界

基线源码为 2993c9a，原安装为 81f0fec（schema 5）。本轮已解除 application 的 Pi-only 活动状态假定，按独立 Pi、Codex app-server 和 DeepSeek ACP 连接状态装配。同进程 Rust＋UniFFI 保持不变，外部 Harness 子进程不是 core Host。当前配置使用 schema 6；旧普通配置直接初始化并覆盖，不备份、不迁移，不改原 Harness 或会话文件。

已实现类型为 pi-1.0.2、codex-0.159.3、dsh-acp-0.2.0-rc.2，分别按精确 regex 校验。Pi 固定 SDK 1.0.2，尚未实现其它 Pi 版本；描述契约允许增加独立版本适配器，不把未验证版本标为支持。版本不匹配在替换当前活动连接前拒绝。添加或编辑非活动实例不打断活动会话。

Codex 使用原生 app-server 创建、恢复、发送、取消、模型选择与增量消息；本机 0.159.3 官方 generate-ts 在临时空 CODEX_HOME 离线生成的类型是接口证据。此版本适配器使用 Responses，不翻译 ChatCompletions。审批与用户问题通过具名交互契约呈现，必须显式回答；不支持的交互明确拒绝，不自动批准。取消仅使用运行时广告的取消／拒绝选项；陈旧或不匹配回复被拒绝。

DeepSeek 固定 @deepseek-ai/dsh 0.2.0-rc.2。SDK 三方法不足以恢复和取消，ACP v1 提供 new/resume/close/prompt/cancel/set_config_option；历史由 huihua 只读提供。ACP initialize 的 agentInfo.version=0.0.1 不是发行版本，因此检测 CLI --version。执行装配用应用自己的 patch 禁用 settings、llm-deepseek 和 llm-deepseek-account 来源，再注入 llm-pi-ai 网关配置，避免原设置优先级使请求绕开网关。凭据仅通过子进程环境传入，不写入 patch。支持原生 ChatCompletions／Responses；Dsh 的 reasoning wire mapping 不能从通用级别列表推定，本轮不注入猜测的映射。

huihua 固定 0.2.0（MIT，Node>=22.18.0），通过公开 providers API 与显式根目录读取 Codex／DeepSeek 会话。原生 ID 用于恢复，应用只投影，不新建会话数据库。包、lock 和完整传递许可证打包，并可由 Mac 原生菜单打开。Codex 与 Dsh 可执行文件由用户安装，不分发其源码或二进制。恢复使用运行时配置的初始模型，并显示提示；不宣称恢复历史模型选择。

原生运行时看到提供商实际 model ID；LLM Gateway 在闲置模型切换时原子替换该 alias 到内部模型条目的映射，避免不同提供商相同 API ID 的歧义。已在途请求保留捕获的路由。配置与凭据仍由 application 装配，AI／gateway 不依赖 Harness。

[Obelisk](https://github.com/tommy0103/obelisk) 的本地来源身份与历史索引有参考价值，但其主要能力是 SQLite 检索、许可为 AGPL-3.0；本轮不引入依赖、源码或第二套会话持久化。

## 责任与验证

root 负责任务文档、运行时 unit／资源装配与独立验收；稳定 Mac/application owner 贯通连接、类型、UniFFI 与原生界面；huihua/import owner 负责包、许可证与 DeepSeek ACP。关键判断曾请求 advisor，但 agent thread limit 阻止创建／唤醒；据官方协议和有界实验完成具体判断，未把候选方案直接当作事实。guides/delegation.md 当前不存在，按已明确的责任边界执行。

已通过 debug 动态库隔离验收：实际 Codex 与实际 Dsh 经本机合成 HTTP 上游共四次请求，验证创建、发送、huihua 历史、恢复、原生 ID／cwd，以及同一 API ID 在不同提供商下准确路由；不兼容版本拒绝且活动连接保持。Pi 合成五次请求覆盖导入、工具续接、编辑 key／ID／地址、原来源文件不变和预览失效。配置脚本验证 schema 6、私有文件权限、显式 key 编辑、参数模板与无备份重置。网关九项 HTTP 原生验收通过。

交互脚本通过 UniFFI→application→stdio 验证显式批准、私密回答、非法／陈旧回复拒绝、取消、进程 EOF 保留已收到内容与再次连接。发现并修复终止错误使所有后续 application 操作反复失败，以及同一轮终止事件丢弃已收到增量的问题。没有新增自动测试、读取真实秘密／会话或调用真实模型服务。

当前 fmt、workspace check／严格 clippy（全部 features／targets）与 bindings no-default-features 静态检查通过。Mac 严格 Swift 类型检查已由源码 owner 完成。恢复后的模型提示已移到原生历史装配之后，隔离复验确认存在。无会话模型切换被明确拒绝；可选路径留空不再被转换成当前目录。干净发行构建、安装与已安装库复核仍待完成，完成前不将 debug 结果视为发行包验收。
