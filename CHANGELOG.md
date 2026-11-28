# Changelog

## [0.1.0-alpha.3](https://github.com/xiaoland/velune/compare/v0.1.0-alpha.2...v0.1.0-alpha.3) (2026-11-28)


### Features

* **conversation:** 支持分叉打开方式并保留编号 ([3dec108](https://github.com/xiaoland/velune/commit/3dec108553081eab6d1f9f38ef170037e7cab6c7))


### Bug Fixes

* **acceptance:** 校验验收子进程退出和检查记录 ([5cd694b](https://github.com/xiaoland/velune/commit/5cd694b75a31d4b46e67eaff7a43169facd082df))
* **mac:** 以系统窗口身份恢复工作区 ([8ad20db](https://github.com/xiaoland/velune/commit/8ad20db1b8c5cc6014b477d923ecdeee6ac551a4))
* **pi:** 避免压缩展示提示误判为编辑冲突 ([b10b39a](https://github.com/xiaoland/velune/commit/b10b39a3b9ef94b9e6308c095dca4c98c928a8fe))
* **remote:** 恢复相邻 Alpha 的远控兼容 ([6f941a5](https://github.com/xiaoland/velune/commit/6f941a5245157769ffcbec1b91b02900b284517e))


### Miscellaneous Chores

* **release:** 准备 0.1.0-alpha.3 发布 ([50fa657](https://github.com/xiaoland/velune/commit/50fa6570ac4bc19aee7388de8366fccf96ebe59b))

## [0.1.0-alpha.2](https://github.com/xiaoland/velune/compare/v0.1.0-alpha.1...v0.1.0-alpha.2) (2026-11-28)


### Features

* **ai:** 接入 Google Gemini 原生 API ([e7d0b8a](https://github.com/xiaoland/velune/commit/e7d0b8a2574911c80119c6ced59eb21fae2f35e6))
* **android:** 对齐 Mac 会话能力（权限模式、推理强度、消息编辑与分叉）并修复 Composer 布局 ([7398a5e](https://github.com/xiaoland/velune/commit/7398a5e66e954547952f2e885be6feacd95001f4))
* **android:** 将会话体验集成到 develop ([c438729](https://github.com/xiaoland/velune/commit/c438729493d271be584c740ec6aef5b35be27434))
* **android:** 接入运行时切换、附件上传、手动刷新、后台任务、Agent协作与排队/介入 ([7699196](https://github.com/xiaoland/velune/commit/76991960a48f555b93921f2fd7c39b234ecf6c8e))
* **android:** 新增外观字号/密度设置、直达新会话与贴合键盘 Composer ([f71a239](https://github.com/xiaoland/velune/commit/f71a2396facee3d34206815c24df14d5664fd8a9))
* **android:** 沉淀结构化 debug 模拟数据资产并同步文档 ([dfb01e9](https://github.com/xiaoland/velune/commit/dfb01e947f5240da3acbb30e55e9811909651f83))
* **composer:** 支持撤回排队消息 ([2c39dde](https://github.com/xiaoland/velune/commit/2c39dde26005a325d5063d17435bed263041ecec))
* **dev:** svc run run-macos-dev 一步构建并启动 Mac 开发版 ([6f855fe](https://github.com/xiaoland/velune/commit/6f855fe60cbaeb6340524146d6c5027284c56ed9))
* **gateway:** 接入 Google Antigravity 订阅 ([7505408](https://github.com/xiaoland/velune/commit/75054080b5f552db5f09695876b6cf44dc41d6ae))
* **mac:** ⇧⌘N 在新窗口中新建会话 ([f8b2b63](https://github.com/xiaoland/velune/commit/f8b2b63a54fa69eaff2cc3ffbf390c616cdbce44))
* **mac:** 列表在任何位置形态一致，停靠可见性收敛为两层 ([09fb6cd](https://github.com/xiaoland/velune/commit/09fb6cd595793682f3f5853450ff63020fb4ffa4))
* **mac:** 取消浮动工具面板，旧布局迁移为独立窗口 ([302b340](https://github.com/xiaoland/velune/commit/302b34072a8631fe6e2ef7a6fdf9f66d70ec1b2e))
* **mac:** 增加应用级界面缩放 ([683abc5](https://github.com/xiaoland/velune/commit/683abc52b73c6f1a9f22aa19a446cce9267e4ac6))
* **mac:** 工作区与会话窗口改由 AppKit 承载和恢复 ([ed01614](https://github.com/xiaoland/velune/commit/ed01614987db13ae78246b2fc7c3703694e5feff))
* **mac:** 底区与区内工具改用原生分栏 ([fc9aebe](https://github.com/xiaoland/velune/commit/fc9aebee32e769e82b27e47d775623d2b5006184))
* **mac:** 抽出可独立检查的工作区布局模型 ([16d0d0d](https://github.com/xiaoland/velune/commit/16d0d0d4282b69f82f51b8354255f30297fd32c5))
* **mac:** 标签栏改由 AppKit 绘制，标签放不下时横向滚动 ([1947907](https://github.com/xiaoland/velune/commit/19479074e0d2f0f0c68668f9b550dafd5e5c5665))
* **mac:** 编辑区网格改用原生分栏并按组保留面板视图 ([fd07fd1](https://github.com/xiaoland/velune/commit/fd07fd1276cc5888e5683674f7b64042e7640624))
* **mac:** 边栏、编辑列与检查器改由 NSSplitViewController 承载，工具栏改为 AppKit ([733417b](https://github.com/xiaoland/velune/commit/733417b6630434ff1d99c18d9af8e74462024df6))
* **providers:** 完成 Antigravity 订阅额度接入 ([951406d](https://github.com/xiaoland/velune/commit/951406d29c16dc7039d67f25daffb028fb406e0d))
* **providers:** 支持手动查询 Antigravity 订阅额度 ([6a6d1ad](https://github.com/xiaoland/velune/commit/6a6d1ada566b1e5818e2852a30ce3f2020de4031))
* **providers:** 支持配置 Codex 订阅客户端版本 ([bd75ce4](https://github.com/xiaoland/velune/commit/bd75ce4ff310ff633e21831aa82f415fa93ca3e6))
* **runtime:** 接入 Antigravity ([c026359](https://github.com/xiaoland/velune/commit/c026359a3e53bcc56bf6d7811b3285c6a7f690db))
* **runtime:** 支持按模型配置自动压缩 ([567ff0b](https://github.com/xiaoland/velune/commit/567ff0b383bb16ce530646a7d258b0b2281ff29e))


### Bug Fixes

* **acceptance:** 由父进程在 worker 退出后删除临时目录 ([d238348](https://github.com/xiaoland/velune/commit/d2383489454e3d596a4192f35d846e6895416a40))
* **acceptance:** 记录外部程序版本时跳过前置警告行 ([7d2fd23](https://github.com/xiaoland/velune/commit/7d2fd23c8c492d4c23d04c7cbb5f4d2796033fd0))
* **android:** 修复 steerTurn 绑定签名、分叉/编辑寻址与修订基线等高价值 review 问题 ([891155e](https://github.com/xiaoland/velune/commit/891155e4899c55c2a73d8f5498c86eb6906a5b74))
* **android:** 独立刷新会话列表和活动状态 ([161cb1e](https://github.com/xiaoland/velune/commit/161cb1eae78e176ff5ba0b2b5808167d41198f25))
* **android:** 编辑消息时保留原消息的图片与文件附件 ([db25289](https://github.com/xiaoland/velune/commit/db252896a5091e1170927181f1bb3775d07812e3))
* **antigravity:** 让新增代码通过 Clippy 门禁 ([ddbfdf0](https://github.com/xiaoland/velune/commit/ddbfdf074b7cad471556679ed948183e8f049cb8))
* **archive:** 加快归档会话的首屏读取 ([c0afdf2](https://github.com/xiaoland/velune/commit/c0afdf2970edc349ecbd6353eda4682c08fdac91))
* **archive:** 省略项目分组内重复的项目名 ([d7a3aae](https://github.com/xiaoland/velune/commit/d7a3aae2dfde3e73e8cd9db6822587d9174a0e47))
* **background-tasks:** 区分离线状态并接通 Codex 面板 ([045bbef](https://github.com/xiaoland/velune/commit/045bbef4418968fefc5e57ae2d613329d5d07147))
* **background-tasks:** 增加手动刷新和就地重试 ([d61a476](https://github.com/xiaoland/velune/commit/d61a4763f1708ae2691a3f10d4dbcbf23aff27db))
* **claude:** 修复压缩前消息编辑发送失败 ([dcf4961](https://github.com/xiaoland/velune/commit/dcf49612dfdadb843869eeb49c945be4a162ff46))
* **claude:** 合入订阅桥接跨重启续聊 ([ccc214a](https://github.com/xiaoland/velune/commit/ccc214a73df77a931edae61a66d2a26873b03578))
* **claude:** 后台任务期间复用进程发送新消息 ([e016508](https://github.com/xiaoland/velune/commit/e016508d77c73fdd3178a7be2da37f96f8137be2))
* **claude:** 恢复历史消息编辑和会话分叉 ([090ab36](https://github.com/xiaoland/velune/commit/090ab36170978cc8eafd224f4f4d3becff1482e5))
* **claude:** 等待原生输入开始回执 ([09d68a4](https://github.com/xiaoland/velune/commit/09d68a4ddc16b4038e6f30cd42afb47ae65e99a3))
* **claude:** 补齐会话自动命名兜底 ([dcae345](https://github.com/xiaoland/velune/commit/dcae345dabfc165f71ee235e5a593151d6efe172))
* **claude:** 让压缩前的会话分叉可发送 ([4390955](https://github.com/xiaoland/velune/commit/4390955b912c1a3933ba11e8863038d9c42fa4ee))
* **claude:** 跨重启保留订阅桥接缓存前缀 ([2569ce5](https://github.com/xiaoland/velune/commit/2569ce5391d773aa6c685d659325eb6af964c908))
* **codex:** 接入原生后台终端观察和单项停止 ([be3d40f](https://github.com/xiaoland/velune/commit/be3d40fc6fb9560132bbdeb0f9653f8d7c48885f))
* **composer:** 即时显示发送状态并恢复失败输入 ([2a06818](https://github.com/xiaoland/velune/commit/2a068185cab2c270bc8bfa36c00592ebf0778408))
* **composer:** 只读会话保留草稿编辑 ([1a1a85e](https://github.com/xiaoland/velune/commit/1a1a85efd310ab6dbb62ca7681d7854b0538f412))
* **composer:** 提交插入消息的执行配置 ([b2892a9](https://github.com/xiaoland/velune/commit/b2892a99f8b36b4469234349efd3d0c12661c311))
* **composer:** 插入本轮沿用活动模型和推理强度 ([929c8e3](https://github.com/xiaoland/velune/commit/929c8e3ad57873a46496fd9d96549985bdcce0a5))
* **continuation:** 让接续压缩等待原生结果 ([64a55bb](https://github.com/xiaoland/velune/commit/64a55bb474c2f8261a1da74cf96573e3fa18f31b))
* **conversations:** 以原生已读回执结算操作 ([74f5685](https://github.com/xiaoland/velune/commit/74f56855e6251c2bb3e55fe5ac6a98d2d8491bc1))
* **conversations:** 用原生消息通知驱动 Mac 列表更新 ([9f57919](https://github.com/xiaoland/velune/commit/9f57919a3fc2595a11669d40842fc9ddb3d046e3))
* **gateway:** 上游 HTTP 403 后尝试下一路由成员 ([a623ebc](https://github.com/xiaoland/velune/commit/a623ebcf268ea0cb0a66d343f88c4d0dc8c41d82))
* **gateway:** 更新 ChatGPT 订阅的 Codex 客户端版本 ([c5a2eca](https://github.com/xiaoland/velune/commit/c5a2eca3f8b8283ecdbf2c123246b337162e3f81))
* **integration:** 修正合并后的回归与 Rust 门禁 ([23307d4](https://github.com/xiaoland/velune/commit/23307d4144b813e93136072666a00465658932af))
* **integration:** 让 Codex 索引证明使用合并后的历史配置 ([081bf9c](https://github.com/xiaoland/velune/commit/081bf9c9f3fb12e78224caba2a6604aa8b383a40))
* **ios:** 接入异步提问并恢复设备构建 ([3126d08](https://github.com/xiaoland/velune/commit/3126d08f287a42ff4e32a4ff59a0a7e5e58bb9b6))
* **live:** 无活动连接的父会话视为已结算 ([4f7746c](https://github.com/xiaoland/velune/commit/4f7746ce6373ea39bce73c14baa8b0a0a41f9f06))
* **mac:** “新建窗口”打开新的工作区，不再复制当前窗口的排布 ([0bf1e01](https://github.com/xiaoland/velune/commit/0bf1e01d3c541117212aa3d8628f71e7d7d4c7f3))
* **mac:** 仅在消息列表显示核心确认的消息 ([ed5f65c](https://github.com/xiaoland/velune/commit/ed5f65ca991414ff4fc9d1cbc44abaacec9501a5))
* **mac:** 以真实数据验收外壳：窗口尺寸、菜单、无障碍与中文强调 ([ccf50fb](https://github.com/xiaoland/velune/commit/ccf50fbeedea4f7a438d8bf506b479ca36f55cf2))
* **mac:** 会话窗口工具栏改用紧凑统一样式以协调图标比例 ([afacbdd](https://github.com/xiaoland/velune/commit/afacbdd4b40556654c16e97787489513687880c4))
* **mac:** 保留后台标签的消息列表并延迟重排 ([a6b99d8](https://github.com/xiaoland/velune/commit/a6b99d82867a8e032cbdf762bcee5f22f2192a5d))
* **mac:** 修复会话列表工具栏项所有权分配 ([aa42506](https://github.com/xiaoland/velune/commit/aa42506c2fdc454156f686acd4e5dd6eb80f35d9))
* **mac:** 分栏不再以自身为代理，修复新建再关闭标签即闪退 ([364d659](https://github.com/xiaoland/velune/commit/364d6593da890e6dc9882f6412deebedb6c69160))
* **mac:** 后台标签跟随设备活动结算 ([d657ed1](https://github.com/xiaoland/velune/commit/d657ed120feb1681a7604b98af367196128fe865))
* **mac:** 启动时自动重试打开会话期间显示加载，而非随即自愈的错误 ([c46266a](https://github.com/xiaoland/velune/commit/c46266a99454cf917ef1f952dbba3f17b79a29f0))
* **mac:** 在读取错误旁提供重试 ([9216760](https://github.com/xiaoland/velune/commit/9216760b3dc70717ec24e7fe89466237819e53fc))
* **mac:** 打开设置时不再自动选中第一个文本框 ([ee8e50e](https://github.com/xiaoland/velune/commit/ee8e50e25eb454eda032263157e15313e431901c))
* **mac:** 提前读取发送后的原生用户消息 ([2cf93d6](https://github.com/xiaoland/velune/commit/2cf93d62420fdf77fcae4e9aaf3a68fb094bd063))
* **mac:** 移除标签栏系统滚动条并修复外壳与窄分屏边界 ([b50f964](https://github.com/xiaoland/velune/commit/b50f964df797fa49deca3bb3958259b255029463))
* **mac:** 稳定恢复会话侧栏工具栏分区 ([f83d363](https://github.com/xiaoland/velune/commit/f83d36369fc51d8c50a9120d412a6699ee5c792f))
* **mac:** 让 Composer 执行配置跟随界面缩放 ([1cb7af2](https://github.com/xiaoland/velune/commit/1cb7af2ec5725f861fad1437fe4075d7ce315f06))
* **mac:** 让打开会话搜索覆盖未加载的会话 ([64cfcec](https://github.com/xiaoland/velune/commit/64cfcec030df342391423be32a065dd8394fdb0b))
* **mac:** 边栏开合不在动画中重建视图 ([f688a6b](https://github.com/xiaoland/velune/commit/f688a6b7c0affee7ba9ba877bc9c510e3d726ad8))
* **mac:** 运行时配置未加载前输入区不显示内部运行时标识 ([8c08830](https://github.com/xiaoland/velune/commit/8c088306a81feab8c11c118158426c60d9c44bc4))
* **mac:** 退出无需等待遥测连接关闭 ([2d37a98](https://github.com/xiaoland/velune/commit/2d37a98c7c096f9c1394beceecb01d9fe37204f3))
* **mac:** 问题标题、读取失败说明与提供商认证状态可读 ([a7d880a](https://github.com/xiaoland/velune/commit/a7d880a7decc8da05cb4873269fd5378573fb689))
* **pi:** await composed prompt acceptance ([a62f4ef](https://github.com/xiaoland/velune/commit/a62f4ef343676cfb6bf5748f68cad9fb8b4ae5a8))
* **pi:** 修复压缩中断后的会话输入与活动状态 ([084ea86](https://github.com/xiaoland/velune/commit/084ea8690f151c7f1eb286fe56cc6556724621a8))
* **pi:** 修复新建会话立即发送时丢失首条消息 ([b267852](https://github.com/xiaoland/velune/commit/b267852571349e16d699568f87d786201506ddae))
* **pi:** 确认清队列实际取消的后台通知 ([832b231](https://github.com/xiaoland/velune/commit/832b231d9f2f377964bbacd3227b124985f0b587))
* **pi:** 避免后台桥接初始化误报 ([5d6c3b5](https://github.com/xiaoland/velune/commit/5d6c3b5631d769e439632589929643251fa09dce))
* **release:** 修复 Alpha 的 Linux 门禁失败 ([d6e1e7a](https://github.com/xiaoland/velune/commit/d6e1e7a9c220bcc368d7b2086b3ae730837f4498))
* **release:** 完整读取 Xcode 版本输出再校验 ([477f5bc](https://github.com/xiaoland/velune/commit/477f5bcaf340552fa723bfaf370289e0d561e6e4))
* **release:** 完整读取 Xcode 版本输出再校验 ([54cc17f](https://github.com/xiaoland/velune/commit/54cc17f21f96de39f8f0ca89bbbe9b4937a1d3c3))
* **remote:** 使用链接签发与打开配对邀请 ([b9cf66e](https://github.com/xiaoland/velune/commit/b9cf66e7cbfa317650569bc5a693d0bdb4bb1bb1))
* **remote:** 减少会话列表等待并释放执行线程 ([7078abf](https://github.com/xiaoland/velune/commit/7078abfded9050008b123210f441343f872ab3cf))
* **remote:** 提前显示安卓会话并修复连接清理 ([8e4bb15](https://github.com/xiaoland/velune/commit/8e4bb1532dc00afa246be6729bb1c984fcb13836))
* **runtime:** 保留后台结算期间的会话派发 ([3e939ac](https://github.com/xiaoland/velune/commit/3e939ac37cefdd37fe95ec0021d62358c7e7d1c2))
* **runtime:** 支持 Codex app-server 0.170 ([7bad70f](https://github.com/xiaoland/velune/commit/7bad70f602b0d29be91fed718fd43621858fabe6))
* **runtime:** 支持 Codex app-server 0.171 ([28a5777](https://github.com/xiaoland/velune/commit/28a5777d3c2976834f5814d7720447fe5bed57f3))
* **runtime:** 隔离 Pi 重命名超期并恢复会话接续 ([b55baf7](https://github.com/xiaoland/velune/commit/b55baf72d4bef39092de01be004eeff91985459d))
* **side-chat:** 恢复侧聊回复与状态更新 ([6c07f1b](https://github.com/xiaoland/velune/commit/6c07f1b6523e5ca50c5b67a019453b87e49332ce))
* **tests:** 补齐 Claude 与 Pi 测试夹具的输入回执 ([8f0eeb2](https://github.com/xiaoland/velune/commit/8f0eeb2a79e975c8ba6001259eb2c1bba541de0c))


### Performance Improvements

* **application:** 消除空闲轮询中的重复快照构建、模型哈希与二次扫描 ([e55b167](https://github.com/xiaoland/velune/commit/e55b167d1a568574dffb8f626d08f52eec3df90f))
* **codex:** 跨启动持久化普通会话索引校验并跳过重复预热 ([928ba2e](https://github.com/xiaoland/velune/commit/928ba2ea864acb734dff912d39c801247a25c85d))
* **codex:** 预热并复用普通会话索引 ([3e41434](https://github.com/xiaoland/velune/commit/3e41434f50aa191ba4507c9da8b74990f71476f8))
* **integration:** 合入 Codex 活动索引持久化 ([6ad5461](https://github.com/xiaoland/velune/commit/6ad546193644c375cef73b9f36c6e43773c90684))
* **integration:** 合入空闲轮询降耗 ([82d5c8b](https://github.com/xiaoland/velune/commit/82d5c8bb2b8260596fd291644354fc8566e44b06))
* **mac:** 真实数据规模下菜单、列表与边栏开合的主线程开销 ([d9be1a4](https://github.com/xiaoland/velune/commit/d9be1a4107194fe32e2839bcbe45e86f3c3f2b68))
* **mac:** 边栏开合动画中不再每帧重算输入区与测量标签 ([38b6203](https://github.com/xiaoland/velune/commit/38b6203aaf04cbf78bb56c5e9e7baf43b3da2762))


### Miscellaneous Chores

* **release:** 准备 0.1.0-alpha.2 发布 ([e83ac7e](https://github.com/xiaoland/velune/commit/e83ac7ece411e6fd372602d1725897cb95bc2ecc))

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
