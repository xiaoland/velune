# 跨运行时统一会话浏览

2026-10-06 用户追加统一列表需求，所有启用运行时的会话在同一浏览器出现，支持 runtime、CWD／project、label／section 分组筛选，按创建及更新时间排序。执行实例不能作为隐式筛选；列表可懒加载或分页。先前加载中菜单可用反馈继续完成，不因新反馈丢弃。

已观察：application.list 合并各配置实例历史并隔离错误，Mac Views 又按 selectedRuntimeID 过滤；ConversationSummary 仅有 updatedAt，Pi helper 已返回 created但 application 丢弃，huihua metadata待核对。当前实例无 enabled 字段。现存schema7有效配置应避免因增量字段添加不必要重置，但不得引入旧schema迁移。

源码稳定owner session_management，负责菜单修复及统一列表贯通，root负责docs／独立验收／安装。public_model_templates提供只读关键契约判断，不作review。用户正在澄清标签来源（原生标签还是Velune列表组织配置）；该部分不提前推定存储设计，其它已授权工作继续。

拟议边界：runtime-neutral summary携带真实可选created/updated与projectmetadata，缺失不编造；展示层显式筛选／分组／排序，保留native identity及所属runtime。启用配置与执行身份解耦，不增加会话DB或复制历史；原生labels能否统一及本机组织偏好待证据和用户决定。

验证使用类型检查、静态检查、release构建与临时HOME合成端到端脚本，覆盖多个实例／项目、时间缺失／同值及排序稳定、运行时来源失败隔离、菜单管理与加载结果顺序。真实配置、凭据、会话不读取，真实服务与GUI由用户验收。完成后clean构建安装0.1 beta.1。

## 契约证据与采用方向

固定版本只读判断（2026-10-06）：Pi header／SDK、Codex session_meta、DSH header 可提供创建时间和cwd，摘要新增可选 Unix 毫秒，未知保持 None。当前 RuntimeInstance 无enabled，新增默认true字段为增量可选配置，不需旧schema迁移或reset，schema7保持。浏览filter/group不依赖执行runtime；相同basename不同cwd不合并，native身份保留实例命名空间。

huihua0.2 scan 无可靠updatedAt，公开read遍历events后才得出更新时间；正确排序需在adapter补取得可靠时间。Pi modified代表lastmessageactivity，不因rename更新。文件mtime不自动等于会话活动时间。Pi entry label 是树节点，不是会话标签；Codex有section原生API但huihua不暴露，DSH未核到标准会话labels，标签来源仍待用户。

## 标签范围澄清

用户回复 label／section 只是举例，非所有 Harness 支持，Codex section 聚合 project；若加入自定义 tag／label／section 则由 Velune 另持久化组织记录。由此本轮必要实施先为 runtime／project 分组筛选、created／updated 排序、全启用实例与加载管理操作。不加入标签 CRUD／空存储框架，不把 Pi tree entry label 或 Codex section 冒充统一会话标签；未来列表组织数据独立于原生会话历史。源码 owner 已同步。

## 当前隔离证据与收口

共享摘要新增真实可选 createdAtUnixMs，Pi SDK／Codex／DSH元数据在adapter规范；huihua list调用公开read取得可靠日期。Runtime enabled作为当前schema7默认true字段，禁用不读来源、不创建会话，禁用当前idle停止执行并清空projection，再启用恢复历史。界面不按执行runtime隐式filter，详情归属保留实例identity，显示菜单提供none／runtime／完整cwd分组、runtime／project显式筛选、created／updated排序和方向，未知时间末尾。

源码 owner 通过实际 UniFFI／Core 多实例浏览、禁用缺Node实例不运行helper或产生失败、禁用当前idle清空、disabled open/create拒绝、再启用历史恢复、schema7缺enabled使用当前默认且配置不重置。实际 AppStore／Transport 脚本通过两个Pi HOME相同native ID切换／恢复，统一列表仍保留，group/filter不改loaded；同basename不同完整cwd分组、time正逆序与未知日期末尾通过。

加载管理扩展验收通过pending目标改名／删除、原loaded改名／删除、加载失败后管理、原生删除失败保留loaded，旧结果不插回。workspace fmt/check／strict clippy和bindings no-default strict clippy、Swift warnings-as-errors及脚本语法通过；源码功能冻结，最后Codex／DSH metadata脚本断言收尾中，尚未新版安装。

Codex 0.159.3／DSH 0.2.0-rc.2 实际运行时原生created／updated非空隔离断言及五次loopback续接通过；全部源码、脚本冻结。最终静态检查包含无默认features产物和所有手工Python／JS入口，无自动化测试。接下来clean构建、安装和root安装包复验。
