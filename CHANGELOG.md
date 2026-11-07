# Changelog

## [0.1.0-beta.1](https://github.com/xiaoland/velune/compare/v0.1.0-beta.0...v0.1.0-beta.1) (2026-11-07)


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
