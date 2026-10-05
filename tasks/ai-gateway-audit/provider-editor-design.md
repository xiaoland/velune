# 提供商编辑器：功能、内容与原生交互

2026-10-05。本设计对应用户最新批准的提供商配置与模型模板方向，替代独立认证建档和全局模型 CRUD。Rust／UniFFI 与 Mac 已按以下契约贯通；实际原生体验仍由用户验收。

## 内容归属与操作

用户配置的是一个提供商以及该提供商可调用的模型。提供商保存名称、协议、服务地址和认证；这些字段都允许编辑。模型保存该提供商 API 的真实 model ID、显示名称、图标、实际能力与限制，以及该协议支持的 reasoning effort 声明。内部记录键仅用于选择和持久化，不进入表单。

模板用于快速填写模型参数。应用模板创建独立可编辑快照，不生成跨提供商调用实体或共享绑定；后续编辑提供商模型不修改模板。保存为模板只复制模型填写内容，不复制提供商认证、地址、路由、内部记录键或运行时来源的私有投影。协议特定 effort 声明不跨协议翻译。模板改动对已有快照的行为以最终冻结契约为准，当前实施按不自动覆盖处理。

| 操作 | 入口与内容 | 成功后行为 |
| --- | --- | --- |
| 新建／编辑提供商 | 提供商列表的添加／编辑 | 保存整个提供商草稿，返回更新后的模型与连接状态 |
| 编辑认证 | 提供商的「连接与认证」 | API key 可显示／隐藏及编辑；OAuth 仍在原提供商上下文登录 |
| 添加模型 | 编辑器模型列表的添加菜单 | 手动创建，或从模板复制后继续编辑 |
| 修改模型 | 编辑器选择模型 | 直接编辑真实 model ID；能力摘要与明确的展开入口可见 |
| 删除模型 | 编辑器模型列表的删除 | 仅修改草稿；保存时清除该条目的无效选择 |
| 保存为模板 | 已选模型的操作菜单 | 给快照命名后登记；不保存认证或改变路由 |
| 维护模板 | 提供商页的「模型模板…」sheet | 最小列表、添加／编辑／删除，不提供调用或路由 |
| 导入运行时配置 | 提供商列表的导入窗口 | 模型直接归于导入的提供商，不要求关联全局模型 |

## 编辑器布局

提供商编辑器使用两栏 `HSplitView`。左侧是原生 sidebar `List`：固定的「连接与认证」项，以及「模型」分组；模型行以显示名称为主、真实 API model ID 为副标题。底部保留添加菜单和删除按钮。右侧仅显示当前选择的内容，避免将连接、认证和所有模型塞进一个长表单。

「连接与认证」使用原生 grouped `Form`。连接部分只有名称、枚举协议和服务地址；认证部分显示实际类型。API key 使用 `SecureField`，眼睛按钮切换为 `TextField`，两种视图绑定同一份编辑值并标记 `privacySensitive`。只在打开当前提供商的认证内容时显式读取该条目的 key，不在列表或全部配置加载时批量读取。读取失败保留明确失败状态，不用空字符串冒充已有 key。协议和地址不因凭据选择而锁定；认证授权目标的改变随用户保存草稿显式生效。

OAuth 区域呈现状态、登录／取消操作，以及只读来源说明。运行时名称可以作为来源信息出现，但不作为认证方式。登录事件沿用现有认证 sheet；启动登录只消费当前提供商的登记来源，不要求用户重新填写 Pi 路径或 Node。

模型内容也是独立 grouped `Form`。首先显示可编辑的「模型 ID」、名称和图标，随后用原生 disclosure 的能力摘要呈现可选的上下文窗口、最大输出和「支持的 reasoning effort」，展开后才编辑次要能力字段。未知、无支持和已声明等级在收起状态仍能辨认。未知元数据留空并明确标注未知；它们不成为原生请求的硬必填条件。可用 effort 集合是能力声明，不代表当前请求选择；本轮不扩展会话 effort API。

模板添加使用菜单或短 sheet 选择快照，手动添加不要求先创建模板。模型的「保存为模板」只需要命名和确认内容；模板维护页复用模型内容编辑组件，不复用提供商连接和认证表单。

窗口尺寸以当前内容约束，不以放大解决密度；次要来源与模板维护采用渐进披露。编辑器固定标题与底部「取消／保存」，右侧内容可独立滚动。取消丢弃普通配置草稿，保存进行一次原子配置操作。OAuth 登录本身可能更新原来源认证，不能把取消普通草稿解释为撤销已经完成的登录。保存失败不关闭窗口，字段和原有 key 值不被空值覆盖。

控件使用系统语义颜色、字体、菜单、列表和表单。不添加品牌皮肤、卡片堆叠、负 padding 或自定义 disclosure 样式。详情中的 disclosure 子内容用完整宽度的 leading `Grid`／`VStack`，保留原生展开控件以及现有无额外 transition 的事务行为。

## 已实施接口

`BindingProviderDraft` 只包含提供商 id、name、protocol、endpoint 与所属 `BindingProviderModel` 数组。模型保存隐藏的 recordKey、可编辑的 providerModelId、nickname／icon、可选 contextWindow／maxOutputTokens／reasoningLevels，以及不透明 adapterMetadataJson；reasoningLevels 为 nil 时未知，空数组时明确不支持。

`saveProvider(gatewayId, provider, authenticationEdit)` 一次提交提供商及其模型，认证编辑枚举明确区分 Keep、SetApiKey(value) 与 Clear。公开配置只返回非秘密的 method、configured、provenance 和 actions；`readProviderApiKey(gatewayId, providerId)` 才显式读取当前条目的 API key。OAuth 登录也以 gatewayId／providerId 进入当前上下文。

`BindingModelTemplate` 保存 name、suggestedProviderModelId、nickname／icon 与可选能力快照；`saveModelTemplate`／`deleteModelTemplate` 维护快照，配置列表返回 modelTemplates。导入选择只包含 providerId 与 candidateKeys。运行时与会话保留隐藏的 modelRecordKey 引用真实提供商模型，不再维护全局模型关系或强制路由。

配置描述不返回秘密。提供商保存请求必须区分「未修改 key」与「明确更新 key」，避免尚未读取秘密时把已有凭据清空；模型和认证配置尽可能同一次保存。API key 使用 application 的私有配置文件保存，权限及原子性由 Rust 承担，不走 Keychain。OAuth 原来源文件和 refresh credential 不复制、不删除。

## 验证边界

先验证具名 UniFFI 接口与全部 Swift 源文件的严格类型检查，再由主执行者进行合成配置、单条 key 读取／修改、模板快照、导入与正常会话路径验收。开发过程不读取真实配置或秘密，不调用真实提供商，不新增自动化测试。原生窗口实际布局和真实服务仍由用户验收。
