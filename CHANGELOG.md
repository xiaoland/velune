# Changelog

## [0.1.0-alpha.1](https://github.com/xiaoland/velune/compare/v0.1.0-beta.1...v0.1.0-alpha.1) (2026-11-21)


### Features

* **analytics:** 统一网关与提供商用量分析 ([1a03fe7](https://github.com/xiaoland/velune/commit/1a03fe7cc2dd772987e43ed43686d87e349d97bc))
* **background-tasks:** 收拢和清理已结束任务 ([3221b31](https://github.com/xiaoland/velune/commit/3221b319d2fc80cccd7246ffc002bd0a3e106680))
* **background-tasks:** 独立呈现运行时后台任务 ([c24043b](https://github.com/xiaoland/velune/commit/c24043b0d858ec33287d94b41bd5f21e7df201ac))
* **collaboration:** 让运行中的子 Agent 优先显示 ([d873a4d](https://github.com/xiaoland/velune/commit/d873a4d777a761b3b004690414b39b5cf0db89e8))
* **composer:** 在限额状态显示 ChatGPT credits 余额 ([b57461b](https://github.com/xiaoland/velune/commit/b57461bc4f1849b35449fd5dfbb7b3d033eb991d))
* **composer:** 支持编辑排队消息的完整指令 ([9c8ec82](https://github.com/xiaoland/velune/commit/9c8ec827a74fb66688d1271a95e0c3c7d1ced933))
* **conversations:** 新增独立归档会话窗口 ([f8a2925](https://github.com/xiaoland/velune/commit/f8a292560ce21801ca638bf3b016ccceea79f5a0))
* **providers:** 接入 ChatGPT 额度权益与提供商分析 ([4ab0d9c](https://github.com/xiaoland/velune/commit/4ab0d9ca9171fdad1dda859c08d7b46919a1b1ab))
* **release:** 集成 Alpha 0.1 候选 ([7760ee2](https://github.com/xiaoland/velune/commit/7760ee27a90e03c94727a31f13f403376efdc19c))
* **remote:** 支持四端自定义中继连接设置 ([297653d](https://github.com/xiaoland/velune/commit/297653dba65435f7d2dde48db8b37421ae3cfc84))
* **runtime:** 统一运行时生命周期并支持主动交接 ([e75e2c1](https://github.com/xiaoland/velune/commit/e75e2c1d280609ba32a4fb05dbdea5e72b5df7d4))


### Bug Fixes

* **android:** 支持运行时异步提问 ([0e1f51f](https://github.com/xiaoland/velune/commit/0e1f51f5a5678363139bf9a02b840856f72df23b))
* **application:** 显式释放应用状态目录锁租约 ([338926d](https://github.com/xiaoland/velune/commit/338926dbb86525c716db39b5261e256137276e8e))
* **claude:** 显示 API 异常终止的原生原因 ([1ff179a](https://github.com/xiaoland/velune/commit/1ff179a46ef27cf8c8e76a0c468b2f03fc988e48))
* **claude:** 首次后台 Agent 事件即关联子会话 ([c614680](https://github.com/xiaoland/velune/commit/c614680b996a5f37990c74b88cb5e6952b594bc2))
* **codex:** 恢复后撤下重连提示并允许忽略 ([1ea2062](https://github.com/xiaoland/velune/commit/1ea206205d43d75c1abe52b8a3f845f8d72b9694))
* **codex:** 支持原生协作消息的上下文导出 ([aa9e16f](https://github.com/xiaoland/velune/commit/aa9e16f5fe218d20ff3f7bacd8f2d125864b46bb))
* **codex:** 识别推理配置与加密 Agent 投递 ([c1ce3d4](https://github.com/xiaoland/velune/commit/c1ce3d481ff6afc0fcad38ba5f732954f5de20af))
* **composer:** 修复排队消息重复显示 ([a220849](https://github.com/xiaoland/velune/commit/a2208495706597e60bb6ba320e2f757a27df8380))
* **composer:** 完善历史消息编辑并修复 Pi 原地接续 ([98e88ec](https://github.com/xiaoland/velune/commit/98e88ecc69e7d0fbb840b91f3b1f1b236a8933ed))
* **composer:** 按 Tab 隔离草稿并修复跨组恢复 ([8bd7929](https://github.com/xiaoland/velune/commit/8bd7929c0874774c336f97802f05b463bb277fee))
* **composer:** 显示排队消息的原模型 ([16cebc5](https://github.com/xiaoland/velune/commit/16cebc587bf95dbe4ba199bc26357c8e5cd386bb))
* **control:** 用有界线程池和截止期限替换无界 thread::spawn ([bb86acf](https://github.com/xiaoland/velune/commit/bb86acf0f855c4d7b744f52fe4c3c1723d07098b))
* **conversations:** 修复关联历史校验并补齐诊断身份 ([0e0ab69](https://github.com/xiaoland/velune/commit/0e0ab698eae9f63795dc3977739f51939545cee3))
* **conversations:** 修复空组分页并复用历史索引 ([1a068b5](https://github.com/xiaoland/velune/commit/1a068b50578fc7daa2e18e478622fc5501fddb4b))
* **conversations:** 加速打开列表外的原生子会话 ([6d64f51](https://github.com/xiaoland/velune/commit/6d64f51131c970f474be46404eb0b9537f9f9f4f))
* **conversations:** 恢复子 Agent 消息列表运行指示器 ([637f536](https://github.com/xiaoland/velune/commit/637f536dadb1a1a017b489a3d6f99396560418b4))
* **dev:** 固化 Windows 更新入口并处理短暂目录占用 ([ffaba34](https://github.com/xiaoland/velune/commit/ffaba34d88596f7cd9032e02f3052db15d799545))
* **diagnostics:** 后台轮询失败不再放大成海量问题与日志 ([66793b9](https://github.com/xiaoland/velune/commit/66793b9e7060182df847e58df9c1b9a8620f14a6))
* **fork:** 从完成边界独立分叉而不等待源执行 ([e082824](https://github.com/xiaoland/velune/commit/e0828247fc0577acac168dfe6bb5acd45be25b9d))
* **fork:** 分叉会话经核心列表刷新进入会话列表 ([4321fd7](https://github.com/xiaoland/velune/commit/4321fd745aed85fb0400f93c59a86a2c91c6d622))
* **fork:** 完善确认、进度、命名和原生兜底 ([1ecfb16](https://github.com/xiaoland/velune/commit/1ecfb16bedb7d271dc9c15da911922a82d9b4977))
* **gateway:** 区分额度耗尽与暂态准入失败 ([9dd1b1e](https://github.com/xiaoland/velune/commit/9dd1b1e816a4cfdc709cde91a4e99cf28b185ddd))
* **gateway:** 在上游响应后恢复半开请求准入 ([66e50b5](https://github.com/xiaoland/velune/commit/66e50b51170363b4395fa9fd63df32698fed1163))
* **gateway:** 自动额度观测不再阻止请求 ([3990286](https://github.com/xiaoland/velune/commit/39902866b02ff6cb050db04bd8a2ab5976a7de54))
* **gateway:** 避免单次断流冷却整个提供商 ([16a721b](https://github.com/xiaoland/velune/commit/16a721b13d6a5dc1bcd59c97a28dd32d33d9d225))
* **mac:** 保留快速打开面板的输入法组字状态 ([37d2553](https://github.com/xiaoland/velune/commit/37d25534b3618c78291328e33cac80303b231ebd))
* **mac:** 关闭窗口后停止回写工作区恢复点 ([042f537](https://github.com/xiaoland/velune/commit/042f53787aa1d782b9ac6b45ea12d616ab33e4ea))
* **mac:** 固定会话列表工具栏的侧栏归属 ([b23542a](https://github.com/xiaoland/velune/commit/b23542a8c6aaaf01dc9862834adcdf14564b9b09))
* **mac:** 完整显示会话列表上限数字 ([ba0b5f4](https://github.com/xiaoland/velune/commit/ba0b5f4370a36f4aa26c83d9c67082605546e64e))
* **mac:** 恢复外部执行的消息列表状态行 ([d0126ee](https://github.com/xiaoland/velune/commit/d0126ee520b9721e5bb207548fba6893c056d2ce))
* **mac:** 显示会话打开失败及重试入口 ([d662241](https://github.com/xiaoland/velune/commit/d66224106330d255f205a8f3f593e14371bcf337))
* **mac:** 等待恢复身份并完整保存窗口布局 ([99b0daa](https://github.com/xiaoland/velune/commit/99b0daacab14fffa76295c8ad836ec7ba5058cb5))
* **mac:** 纠正会话列表原生工具栏的分区 ([fd52c08](https://github.com/xiaoland/velune/commit/fd52c082bd14b66ec30ef1218b79889efdd9295d))
* **mac:** 统一额度检查编译边界并稳定原生验收 ([88c81e9](https://github.com/xiaoland/velune/commit/88c81e9bb882304e47a8e08b58498a4371234d91))
* **mac:** 转录持久标签在每次配置时重读字体，不再保留创建时的字号快照 ([efde043](https://github.com/xiaoland/velune/commit/efde04307efbba0c38c2a999491be3862b251e35))
* **mac:** 释放业务回执等待期间的传输长锁 ([68136fb](https://github.com/xiaoland/velune/commit/68136fb8c0968059120060e7a6de525bfb2845ce))
* **mac:** 首次历史读取未就绪时继续打开原会话 ([2c01f4a](https://github.com/xiaoland/velune/commit/2c01f4a989905508cc7f01b03a918d2d19bc88a6))
* **pi:** 修复子 Agent 命名与续作活动映射 ([6105d43](https://github.com/xiaoland/velune/commit/6105d433e6cc16988de283ed6de6b42d080525f1))
* **pi:** 原子读取会话快照与来源身份 ([9d40732](https://github.com/xiaoland/velune/commit/9d407328f5889c21097f5e4d5bc816d4aac94e8d))
* **pi:** 将后台任务通知按原生顺序归入工作过程 ([9198334](https://github.com/xiaoland/velune/commit/919833451eb0288fe67655730e8addfaead21702))
* **pi:** 将子 Agent 请求与通知按原生顺序归入工作过程 ([f0caf8c](https://github.com/xiaoland/velune/commit/f0caf8c1d87f2f03eeb6552887e8fac3fc1d24ba))
* **pi:** 恢复执行时保留已显示的历史消息 ([dad5b03](https://github.com/xiaoland/velune/commit/dad5b039fbe496f76f51f72a8daaa23e095c4615))
* **pi:** 补齐运行时注入消息与摘要展示 ([7955dd0](https://github.com/xiaoland/velune/commit/7955dd0c6edf97b738d4856060e25305d65161c2))
* **quota:** 明确额度观测未覆盖全部额外额度 ([d8421d4](https://github.com/xiaoland/velune/commit/d8421d41e77c4f37f7fead2543d299637c4f7f3b))
* **release:** 固定 Android 验签工具并保留失败证据 ([7438709](https://github.com/xiaoland/velune/commit/74387091129fe5c82651a6ec2ba0fa49296e1701))
* **release:** 固定 Android 验签工具并保留失败证据 ([8598866](https://github.com/xiaoland/velune/commit/8598866ee3a9047ad57186aee8b3eab24bc37fda))
* **release:** 补齐 Mac Tunwg 对应源码 ([c15d873](https://github.com/xiaoland/velune/commit/c15d873ad408d46ed8a16f2874038485ff6e27d3))
* **runtime:** 修复接续到 Codex 后的发送与输入目标 ([0f2bcb6](https://github.com/xiaoland/velune/commit/0f2bcb69a7e0c5e199b65f9b3aa1042eada16dbd))
* **telemetry:** 有界写入本地日志并保持问题历史顺序 ([85e0f3b](https://github.com/xiaoland/velune/commit/85e0f3b63e27ab7b332b84d804fd37799418b62b))
* **transcript:** 保留后台通知与最终回复的原生顺序 ([d4b2d5c](https://github.com/xiaoland/velune/commit/d4b2d5c4dfc8c857b1f6457f038c196facd52e83))
* **transcript:** 将 Pi 工作通知显示为工具消息 ([940b2d5](https://github.com/xiaoland/velune/commit/940b2d59b0565ea5a46dedcffcfc6a005ec74e1c))
* **windows:** 修复空白标签恢复输入时的运行时目标 ([ea11c08](https://github.com/xiaoland/velune/commit/ea11c084378ee34a471576f67fa3df8895f28aed))


### Performance Improvements

* **adapter:** 减少发送与恢复路径的重复工作 ([c1590cd](https://github.com/xiaoland/velune/commit/c1590cdc52874d02a0a8560d58554cc710fedfd1))
* **conversations:** 减少归档列表的历史读取 ([87644dd](https://github.com/xiaoland/velune/commit/87644dd9876594d93539b198c6b40d499058182a))


### Miscellaneous Chores

* **release:** 准备 Alpha 0.1 发布 ([a4af551](https://github.com/xiaoland/velune/commit/a4af5510856a1364e7c6cb5e23ec7a848b9ea00c))

## [0.1.0-beta.1](https://github.com/xiaoland/velune/releases/tag/v0.1.0-beta.1) (2026-11-07)


### ⚠ BREAKING CHANGES

* 累积型用户状态容忍未知持久化字段
* 删除没有平台调用的导出及其背后的核心命令
* 阶段 5，修正测试偶发失败，硬切旧数据兼容，收紧认证事件与提供商导入
* **application-api:** 会话用量由核心汇总，Mac 与 Windows 状态行共用
* **mac:** AppStore 拆为 DeviceStore 与 ConversationStore
* **windows:** 直接使用按 crate 生成的 C# 契约类型，阶段 3 完成
* **application-api:** Command 变体统一为动词短语，C# 命名空间移出 Velune
* **android:** 追平按 crate 分包的 Kotlin 绑定，List 变体改名 ReadConfiguration
* **ios:** 追平合并后的 UniFFI 契约
* **mac:** 删除 Mac 映射层，额度状态携带 QuotaObservation
* **mac:** 直接使用生成的契约类型，删除逐字段复制的 Mac 模型
* **application-api:** 分析记录直接使用 Protocol，删除 AnalyticsProtocol
* **bindings:** 遥测与远控类型直接派生 UniFFI，删除 JSON 转换
* **bindings:** 契约类型直接派生 UniFFI，删除 Binding* 镜像

### Features

* **ai:** 接入 ARK Coding Plan 独立额度认证 ([7adc650](https://github.com/xiaoland/velune/commit/7adc6505122bd48d22eae1872aeb62058d6dc0b4))
* **ai:** 接入 Kimi Coding Plan 订阅 ([1444d56](https://github.com/xiaoland/velune/commit/1444d56c871038ffaf0f4ba0d2e67baaae8ad4b3))
* **ai:** 接入 RelayKit 协议转换 ([ff2aa83](https://github.com/xiaoland/velune/commit/ff2aa838c62eca5d7f5fb5094ee251fd783dcee9))
* **ai:** 支持独立 Codex 订阅聚合与故障切换 ([dd5683d](https://github.com/xiaoland/velune/commit/dd5683dcd457e679f7b9b09687ae6cc1cae1bed2))
* **android:** 接入最小 Compose 远控端 ([235b62b](https://github.com/xiaoland/velune/commit/235b62b0f506a08c00f5580719e8486fc55ffc12))
* **application:** 资源模板库与受管同步，含 UniFFI 入口 ([bed351a](https://github.com/xiaoland/velune/commit/bed351a637d6eb93a1b4c31512a2b2b93fedeae5))
* **clients:** 统一 token 单位显示与输入 ([0ae7dd4](https://github.com/xiaoland/velune/commit/0ae7dd4f4c2de72d429713087a5a1cbe0d3b8cfb))
* **composer:** 完善工作目录选择与命令入口 ([1324512](https://github.com/xiaoland/velune/commit/1324512ab07d3e17c59f08f86b972513b2449a4c))
* **context:** 接入主动上下文压缩与桌面命令 ([ba47054](https://github.com/xiaoland/velune/commit/ba4705414dd076f2593b0d3f27c0e399711c2471))
* **conversations:** 恢复 xURL 导出并合并拷贝菜单 ([b9892ce](https://github.com/xiaoland/velune/commit/b9892cebcc81566341df18a297758dc050c94900))
* **core:** 资源库待同步自动补写、差异接口与 MCP 工作目录／超时 ([4a07d60](https://github.com/xiaoland/velune/commit/4a07d60d542b78c1ce738d9fd2faf7b97e2ecbb3))
* **dev:** 用 SVC 管理遥测环境并补完可用设备验收 ([a97d461](https://github.com/xiaoland/velune/commit/a97d461903eefd3824f3c8dbf0d1bda87806c2a8))
* **fork:** 会话 fork 与消息编辑运行中可用，关联会话走翻译分叉 ([bbe3a7b](https://github.com/xiaoland/velune/commit/bbe3a7b7d4357340672c4d317066109b65893bfa))
* **gateway:** 入口拒绝写入请求分析并补充请求体诊断日志 ([afe817c](https://github.com/xiaoland/velune/commit/afe817cdde2377456a7795ceaa9d4e26ca99023f))
* **gateway:** 支持订阅智能调度与额度准入 ([8616bd0](https://github.com/xiaoland/velune/commit/8616bd0a95dabc9e11db1cf63203a497a2f38c57))
* **ios:** 完善 iPad 工作台与原生交互 ([d3ef18f](https://github.com/xiaoland/velune/commit/d3ef18f96ed1ce0b85571ddf0ac17510facb0c08))
* **ios:** 新增 iPad 与 iPhone 基础远程控制端 ([ff1e1b8](https://github.com/xiaoland/velune/commit/ff1e1b878e3b671827b0af430ebed07c1bfabfc8))
* **mac:** / 与 @ 列表浮在输入框上方，不撑高输入区 ([7b933d5](https://github.com/xiaoland/velune/commit/7b933d536cb557ce9669dd4bf20a1c7f88f8bd6a))
* **mac:** Agent 运行时设置页主从布局与用户指令管理 ([f39466b](https://github.com/xiaoland/velune/commit/f39466b4cabc600c9534f38d40ffcb1237b60c00))
* **mac:** composer 改为原生多行文本框样式 ([e13fc55](https://github.com/xiaoland/velune/commit/e13fc55dc9d6b4a6522fa2568e497fbcef1e0d01))
* **mac:** 会话菜单可停止、继续，并在 sheet 中发送消息 ([58e43fd](https://github.com/xiaoland/velune/commit/58e43fd597ab6f31e0d0e7aac7599bf6869ed47f))
* **mac:** 全键盘操作：发送方式、插入本轮、命令面板与 / @ 补全 ([d6b822c](https://github.com/xiaoland/velune/commit/d6b822ccadcab7f79e16b8a8ea2b0ca690e3392a))
* **mac:** 全键盘操作：行内审批卡片、消息与会话列表按键、速查表与全局快捷键 ([5ad8392](https://github.com/xiaoland/velune/commit/5ad839231941c18b479e9331a668b81e114e5054))
* **mac:** 去掉输入区 ↑↓ 调出之前的消息，改为消息下方悬停出现的复制按钮 ([c845148](https://github.com/xiaoland/velune/commit/c84514820034c53278ea7ebcefacc14431f9552b))
* **mac:** 支持构建并安装 iPad 应用 ([4dff009](https://github.com/xiaoland/velune/commit/4dff009f0200ac10ec25e3ad29f615c5c0f2c26a))
* **mac:** 检索别名：命令面板、/ 命令与下一轮选择按英文名与同义词匹配 ([a1cb13f](https://github.com/xiaoland/velune/commit/a1cb13f360433638a3255bd0dfd10ae489f23b93))
* **mac:** 模板库与受管同步界面 ([27dc812](https://github.com/xiaoland/velune/commit/27dc8127e317507b2f1b6ef78d61e60d37e275c1))
* **mac:** 消息菜单拷贝整条消息并在列表末尾留白 ([6c764bb](https://github.com/xiaoland/velune/commit/6c764bbfb41f41078b6e32a21c4c8432e83118e6))
* **mac:** 设置入口直达对应页、新建会话改为空会话、折叠区间与输入跳动修正 ([5e22a54](https://github.com/xiaoland/velune/commit/5e22a542ca7bc0653db4bee76bf8b0fb8f9756cf))
* **mac:** 运行时编辑内加入资源入口；记录全局资源库与梯级覆盖决定 ([ee92418](https://github.com/xiaoland/velune/commit/ee92418838ebfeda3e722924b1bada59b8bebf04))
* **mac:** 重组模型表单层次，额外请求参数独立披露 ([68b3b37](https://github.com/xiaoland/velune/commit/68b3b373d6e81c7a93ed7a9be2929b8c54f77a37))
* **quota:** Mac 额度节奏、余额、历史与提醒；Windows 同步与文档 ([97b69b6](https://github.com/xiaoland/velune/commit/97b69b6e7c1595a0745e5d37a2279c32e3007416))
* **quota:** 额度节奏调度、官方账户余额、额度历史与提醒（Rust） ([932f244](https://github.com/xiaoland/velune/commit/932f244d5fd884da905385561cf89fbb15133568))
* **release:** 准备首个公开 Beta 的发行与数据兼容基础 ([710418f](https://github.com/xiaoland/velune/commit/710418f5ad1ce2a3708821da35c493ede6d9faf1))
* **release:** 完成首个 Beta 范围与安装更新准备 ([92540c5](https://github.com/xiaoland/velune/commit/92540c52a7aabcc149fca0c178915f23c8087f78))
* **release:** 建立首个公开 Beta 的版本与发行基础 ([6ff6c67](https://github.com/xiaoland/velune/commit/6ff6c6759500eca449ce35108ad6bee1a4af2888))
* **release:** 接入 Windows 安装更新并补充 iPad 自签安装说明 ([21e34db](https://github.com/xiaoland/velune/commit/21e34dbc0e1dd848d425a9e8f4548e29b4bb317f))
* **release:** 集成首个公开 Beta 的发行与兼容基础 ([710418f](https://github.com/xiaoland/velune/commit/710418f5ad1ce2a3708821da35c493ede6d9faf1))
* **remote:** 增加 Android 网络与多路径观测检查点 ([7f552cc](https://github.com/xiaoland/velune/commit/7f552cccdb22c2c031977aa01b0e7acda97bd897))
* **remote:** 实现 v2 库契约并收回平台接入范围 ([4821ddf](https://github.com/xiaoland/velune/commit/4821ddf4c2bd51243fa80baf4782693259136940))
* **remote:** 实现完整业务远控契约与 iroh 接入 ([15399d1](https://github.com/xiaoland/velune/commit/15399d1fcefde512cc0cda5b9944dfe423b9f4b5))
* **remote:** 实现统一寻址、连接恢复与设施代际 ([2b874b3](https://github.com/xiaoland/velune/commit/2b874b3a27c2a8345f7a6f36efd7f4e78d26c530))
* **remote:** 将稳定服务与网络代际分离 ([3f36613](https://github.com/xiaoland/velune/commit/3f366139258044af35cd139f63050d1821082eaa))
* **runtime:** canonical 资源定义与按运行时渲染 ([62a95fe](https://github.com/xiaoland/velune/commit/62a95fea19029da81de01ca67cdc24dd76e703c1))
* **runtime:** 支持 Claude、原生模型与 SSH 执行 ([ea5f507](https://github.com/xiaoland/velune/commit/ea5f5072cf191bb7e1dd119bc96f0f90e71bdcd5))
* **runtime:** 支持停止后无新消息恢复 ([a55c064](https://github.com/xiaoland/velune/commit/a55c06463053618913e8bdaf641d45f67cd85daa))
* **runtime:** 支持跨 Harness 子 Agent 委派 ([e05c604](https://github.com/xiaoland/velune/commit/e05c604a2308a70f840aa33e76206d950e52f664))
* **runtime:** 新增通用 ACP v1 接入 ([cc97cd5](https://github.com/xiaoland/velune/commit/cc97cd502106e4c4fcded91493f8f5366f250041))
* **runtime:** 管理原生指令、MCP、技能与子 Agent 定义 ([a9446a3](https://github.com/xiaoland/velune/commit/a9446a3d23a5ddff389f7cc23cef02e5dc7ce530))
* **telemetry:** 接入四端原生设置、SDK 与开发启动 ([4288414](https://github.com/xiaoland/velune/commit/42884145bd82a5ca973ebe6cf8e39a7fd65bf91b))
* **telemetry:** 添加默认关闭的 OTLP 配置与代际 SDK 基础 ([08aa284](https://github.com/xiaoland/velune/commit/08aa284fee83d9acfa2f8b0239e1a2ec7d98174a))
* **telemetry:** 贯通 FFI、远控执行与网关诊断 ([1c77962](https://github.com/xiaoland/velune/commit/1c779625ae1001fe1fd13bbdad720aaa89b0d754))
* **transcript:** 回复只取末条 assistant 消息的末尾正文，思考与工具归入工作过程 ([2a5187a](https://github.com/xiaoland/velune/commit/2a5187a335e884c9ccf4322990518e95627c5497))
* **windows:** / 与 @ 列表浮在输入框上方，不撑高输入区 ([17bb2d3](https://github.com/xiaoland/velune/commit/17bb2d3cd0f0b2ce1e6fc5ff484d156acd56ee7b))
* **windows:** 全键盘操作：命令面板、发送方式、行内审批卡片、消息与会话列表按键、速查表与全局快捷键 ([02d86a5](https://github.com/xiaoland/velune/commit/02d86a50d80f824cff9c91037874c5047ee864a2))
* **windows:** 去掉输入区 ↑↓ 调出之前的消息，改为消息下方悬停出现的复制按钮 ([7ab1c11](https://github.com/xiaoland/velune/commit/7ab1c117a6dfe326db9c32d79cb95e4a87b65872))
* **windows:** 检索别名：命令面板、/ 命令与下一轮选择按英文名与同义词匹配 ([dd7a03c](https://github.com/xiaoland/velune/commit/dd7a03c78d9e0bdd75be762dd79a15dca309e1de))
* **windows:** 额度节奏提示、账户额度开关、额度历史与提醒通知 ([2f19c6c](https://github.com/xiaoland/velune/commit/2f19c6c5e0aa76d6cffe8019603e2cddf63b6c37))
* **workspace:** 收拢会话输入、运行时与工作区体验改进 ([9567b02](https://github.com/xiaoland/velune/commit/9567b026cb2f1df4c4541cb5b8a66c594bbf8fed))
* **workspace:** 独立设置用量显示并支持 Codex 原生项目 ([8b9b9e5](https://github.com/xiaoland/velune/commit/8b9b9e52485905c49ac7c20d21ac3eccaac8da61))
* **xurl:** 支持跨平台会话深链与 Agent 查询续聊 ([4e2a9f2](https://github.com/xiaoland/velune/commit/4e2a9f28b54f635368354817240334d7861dd3ad))
* 从运行时原生条目导入为模板 ([ac3116d](https://github.com/xiaoland/velune/commit/ac3116d869c27442fb7e8e9a1b0a5bfc3df550ee))
* 侧聊（Claude Code 侧问、Codex 临时 fork、Pi 内存会话）与 Mac 侧聊面板 ([c7d8cbb](https://github.com/xiaoland/velune/commit/c7d8cbb8318bc18d7aa5070a5386addd76cfee9b))
* 大纲按钮行为设置、就地折叠动画与按会话保存的 composer ([e52848a](https://github.com/xiaoland/velune/commit/e52848a5b46d25f54c4ca223b9d83782e24dcb6b))
* 资源库改为以资源为中心，保存即同步 ([2a024a4](https://github.com/xiaoland/velune/commit/2a024a49c7c1ec97cff2f02c5ce540e34b24de45))


### Bug Fixes

* **agent-runtime:** 允许 Pi 扩展注册并约束实际模型派发 ([0ba2c14](https://github.com/xiaoland/velune/commit/0ba2c14a302293c7c8fc25b099df1bb86ae0c97f))
* **ai:** 修正订阅流协议并完成真实双账户验收 ([2d852b9](https://github.com/xiaoland/velune/commit/2d852b9f6212e25923896e9c2223d6a2a406d2dd))
* **app:** 修复 Pi 启动等待并保存会话浏览偏好 ([510d9e0](https://github.com/xiaoland/velune/commit/510d9e0b6b5f58ec079eb822c4bcdfbcb4423c24))
* **app:** 简化会话输入并补全 Pi 失败诊断 ([508c7c4](https://github.com/xiaoland/velune/commit/508c7c4b3fc8e15a8c24628d7b5fd91788a05611))
* **claude:** 使用 Velune 自有入口标识 ([5bb2c9c](https://github.com/xiaoland/velune/commit/5bb2c9ce0ce6690ba6d3861dc7728218a22e061c))
* **codex:** 保留实时与历史回合的具体错误原因 ([0b06c29](https://github.com/xiaoland/velune/commit/0b06c29f2acef3a76ce8cd62270d35b4bac3d238))
* **composer:** 分离内容与目标配置并完善执行和导航体验 ([d9d2b9d](https://github.com/xiaoland/velune/commit/d9d2b9d2257b4c24bc291b9925b9cebdf86cf685))
* **composer:** 统一发送、排队与原生继续交互 ([6df0bf1](https://github.com/xiaoland/velune/commit/6df0bf14faa8ea36223735cbc1bba8011781cf6b))
* **conversation:** 修复折叠范围和原生消息列表性能 ([8df749b](https://github.com/xiaoland/velune/commit/8df749b1277ad3ac8892c92f32340ec4b5fbdf8d))
* **conversation:** 恢复消息编辑与关联分叉行为 ([82d8b9b](https://github.com/xiaoland/velune/commit/82d8b9b0185fcbb343c9f74bf307641ea3971206))
* **conversation:** 松绑关联会话加载的防御性校验 ([3e80891](https://github.com/xiaoland/velune/commit/3e8089116ea188d96511f639506c33dd0780545c))
* **conversation:** 稳定消息投影并修复原生折叠 ([25f2e3a](https://github.com/xiaoland/velune/commit/25f2e3a58186ad1130ba9a594f945b465bee94dc))
* **conversation:** 隔离 Tab 会话并修复阅读导航 ([d328c25](https://github.com/xiaoland/velune/commit/d328c252020181c213a84e2faf4a61dd4d1454ca))
* **mac:** 修复拖放数据回调导致的退出死锁 ([4322a7c](https://github.com/xiaoland/velune/commit/4322a7c18293cc44d7d6796a2895319e535ab304))
* **mac:** 删除会话残留、composer 默认模型、表格间距、运行指示与手动重新读取 ([0ad56f1](https://github.com/xiaoland/velune/commit/0ad56f18aef3d3742f59e6aeb41a6c2e545e0820))
* **mac:** 回收单元格在 prepareForReuse 之后隐藏，消除展开时的兄弟行残影 ([720c228](https://github.com/xiaoland/velune/commit/720c2287313fc943070bdd6ce1f8d8d2e61917d4))
* **mac:** 复制按钮只放在用户消息与每轮最后的回复下 ([bdcc318](https://github.com/xiaoland/velune/commit/bdcc318262e249d0b9d2a12bae487f9803c05e57))
* **mac:** 恢复工作过程左对齐 ([55d70fd](https://github.com/xiaoland/velune/commit/55d70fd0007d6ccdc7803444255aa058edb5a385))
* **mac:** 整顿主窗口与分析界面的操作层级 ([d2bfb0b](https://github.com/xiaoland/velune/commit/d2bfb0beae3dce8c6d88afb2a361ce7f60c2c1dd))
* **mac:** 校验工作区恢复候选后再选择回退快照 ([6cd38cf](https://github.com/xiaoland/velune/commit/6cd38cfae5dd0b1efbd3f7221fe77c7bc5ef6fc9))
* **mac:** 物化行前提交视口内行高，展开工具消息不再覆盖下方行 ([71e0575](https://github.com/xiaoland/velune/commit/71e05758801d62c2f7fa76fbb3609fb30273dfbd))
* **mac:** 稳定消息尺寸、阅读锚点与连续滚动 ([4c5f4ed](https://github.com/xiaoland/velune/commit/4c5f4ed912005041aea7ca64187780fddf921a23))
* **mac:** 设置窗口各页统一尺寸，切换页签不再跳动 ([f66119c](https://github.com/xiaoland/velune/commit/f66119cf831938023807881ce67573eeb91dbe40))
* **migrations:** 保留匹配的 Codex 旧版本并完成真实升级 ([afe431e](https://github.com/xiaoland/velune/commit/afe431ee2054af0350f6a9dc160122a5d29e6699))
* **migrations:** 补齐旧速度字段与输入格式升级 ([765b8f4](https://github.com/xiaoland/velune/commit/765b8f499e9c078e932ff17418549829d1229c46))
* **network:** 清理运行时遗留进程并增加持续网络取证 ([f220071](https://github.com/xiaoland/velune/commit/f220071613c9ad2155ebabc16951c85d05b4f068))
* **pi:** 投影工具结果中的图片块到会话消息 ([898748c](https://github.com/xiaoland/velune/commit/898748c1b06d3ac4a1c8c069fc0ecf182838c447))
* **quota:** 完善账户限额与重置时间显示 ([d2367ec](https://github.com/xiaoland/velune/commit/d2367eca34e022ca8188bcd333b1584111b33f53))
* **quota:** 接通 Claude Code 原生登录额度 ([bb3f86b](https://github.com/xiaoland/velune/commit/bb3f86bdc7513db453807cb280a15d39f3475409))
* **release:** 对齐 Velopack 原生渠道资产名称 ([c9df212](https://github.com/xiaoland/velune/commit/c9df2124fed19dc450a68a2bb442a5054730ec51))
* **remote:** 为 mDNS 装配 LAN 接口并管理发现代际 ([f0df0ca](https://github.com/xiaoland/velune/commit/f0df0ca7a7910ac858504a59d84e23657daca26c))
* **remote:** 修复重新配对恢复并接入本地控制 CLI ([636d5b4](https://github.com/xiaoland/velune/commit/636d5b499d3cc5436e4d4176b1a32c9701e02c7a))
* **remote:** 兼容 Android 沙箱的身份原子发布 ([0004d53](https://github.com/xiaoland/velune/commit/0004d5300aabdd2f70804d3d68b9605d6e162329))
* **remote:** 区分 mDNS 失败阶段并观测真实发布 ([b212e81](https://github.com/xiaoland/velune/commit/b212e81f7723173ad25f00e12b6d930bf407e8f5))
* **runtime:** 删除排他网关政策和任意业务门禁 ([a24b9ee](https://github.com/xiaoland/velune/commit/a24b9eef9517c2935517a5039e6e12fa8edbeb52))
* **runtime:** 补齐原生错误映射并隔离适配器生命周期 ([882d061](https://github.com/xiaoland/velune/commit/882d0614c4580d766bbc7e001886ecaf25b1093d))
* **telemetry:** 清除 Apple 设置旧结果并完成 iPad 补验 ([719e0b1](https://github.com/xiaoland/velune/commit/719e0b17675327135e3ce502e38d34c4468175ff))
* **windows:** 复制按钮只放在用户消息与每轮最后的回复下 ([cb246bd](https://github.com/xiaoland/velune/commit/cb246bd25074530ba75e79f59ba7b5a92a8cedbd))
* **windows:** 按最新会话活动阻止过期恢复入口 ([6c68cf3](https://github.com/xiaoland/velune/commit/6c68cf3746d0356ce66f60a9ebc86c03a6d57ac4))


### Performance Improvements

* **runtime:** 增量列出 Pi 历史并在打开时复用列表成员关系 ([88ad6e6](https://github.com/xiaoland/velune/commit/88ad6e69201f5162c103d3245a16207b7dde8e61))
* **transcript:** 增量交付快照并按需读取大段推理与工具输出 ([af265fc](https://github.com/xiaoland/velune/commit/af265fc21fc16831a809312ec27aa6be0d084244))


### Miscellaneous Chores

* **release:** 提升首个公开 beta 源码至 main ([03ab14a](https://github.com/xiaoland/velune/commit/03ab14a46abd7728834682b66074d9750017eaae))
* **release:** 确定首个 Beta 的 Mac 与 Android 发行范围 ([2aba311](https://github.com/xiaoland/velune/commit/2aba311989c13b3ece556240d63c7891fb4dc5a4))


### Code Refactoring

* **android:** 追平按 crate 分包的 Kotlin 绑定，List 变体改名 ReadConfiguration ([1e7b53d](https://github.com/xiaoland/velune/commit/1e7b53d1d451aa79b0c0f3e1c0ad7ea8f48fd78f))
* **application-api:** Command 变体统一为动词短语，C# 命名空间移出 Velune ([7d82723](https://github.com/xiaoland/velune/commit/7d827231f6c0757f8f6bd31a6da03ce64e534963))
* **application-api:** 会话用量由核心汇总，Mac 与 Windows 状态行共用 ([36ec90c](https://github.com/xiaoland/velune/commit/36ec90c8e1a72813b0c08730289b447e7a14962e))
* **application-api:** 分析记录直接使用 Protocol，删除 AnalyticsProtocol ([0c0f717](https://github.com/xiaoland/velune/commit/0c0f717b0cf049054b647aad59f7f738e087ecc7))
* **bindings:** 契约类型直接派生 UniFFI，删除 Binding* 镜像 ([d108af1](https://github.com/xiaoland/velune/commit/d108af1a58ad85dd72112476f210f89ad9a941c4))
* **bindings:** 遥测与远控类型直接派生 UniFFI，删除 JSON 转换 ([0026f43](https://github.com/xiaoland/velune/commit/0026f43f6050ed45c589c322a527a9434e1693ca))
* **ios:** 追平合并后的 UniFFI 契约 ([447b912](https://github.com/xiaoland/velune/commit/447b9123e66096c8357034c41dacc0a54fdd1529))
* **mac:** AppStore 拆为 DeviceStore 与 ConversationStore ([1da72a6](https://github.com/xiaoland/velune/commit/1da72a6ebfa04fca383326d008b0012c47254747))
* **mac:** 删除 Mac 映射层，额度状态携带 QuotaObservation ([b8884df](https://github.com/xiaoland/velune/commit/b8884dfa6a64ea16e031880bf69d2eb520fe8c24))
* **mac:** 直接使用生成的契约类型，删除逐字段复制的 Mac 模型 ([ba8f5fb](https://github.com/xiaoland/velune/commit/ba8f5fbc1d95c7cc0002757140f03864d2444a5c))
* **windows:** 直接使用按 crate 生成的 C# 契约类型，阶段 3 完成 ([1893006](https://github.com/xiaoland/velune/commit/18930068d59542183ec1f69c874e9b4b8da783fe))
* 删除没有平台调用的导出及其背后的核心命令 ([6891edb](https://github.com/xiaoland/velune/commit/6891edb6b98a6a958603045e7f3fc5abbcda367e))
* 累积型用户状态容忍未知持久化字段 ([32d090e](https://github.com/xiaoland/velune/commit/32d090edfd1f6bb610a08bda6ac932c429c38607))
* 阶段 5，修正测试偶发失败，硬切旧数据兼容，收紧认证事件与提供商导入 ([a0e1192](https://github.com/xiaoland/velune/commit/a0e11921860d11854be8e7c6c00c55f8bfad4cf5))
