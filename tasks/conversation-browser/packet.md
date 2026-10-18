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

## 新建到历史的时间连续性补验

第一次 clean 构建已通过但未安装。收口检查确认 Codex thread/start 提供秒级 createdAt／updatedAt，旧NativeSession.new投影丢弃它们，且此时尚无可浏览rollout，单靠historyread不够。本轮继续补齐原生API秒→毫秒；历史header可能有更细精度，应保留真实值，不为测试相等而量化历史。连续性要求为同一创建事件、不丢回None、不以续轮时钟覆盖，公开来源精度不同允许精化。DSH ACP new无创建字段，若原生header已flush可通过source read补齐；无值不得以当前时钟或mtime猜测。

实际新建／打开容差验收揭示 huihua0.2 将 Codex session_meta 外层event.timestamp优先作为metadata.creation，它可能是延迟写入时间，payload.timestamp才是原生创建。固定adapter需通过公开system event的payload采用真正session creation，不扩大容差或量化掩盖。验收加入首次turn前延迟作为判别，证明创建值不随写入漂移。DSH new后原生header已flush，现来源adapter读取可取得真实创建值；不引入新的持久格式解析器。

新建时间补修最终冻结：Codex API秒级创建转换／公开session_meta.payload毫秒值使用、DSH已flushheader补齐通过。首turn前1.2秒延迟、new→send→open→continue证明同创建事件不漂移，DSH精确值保持；五次loopback。相关原生管理回归及strict静态全部通过。未量化历史或引入clockfallback，第一次产物不安装，后续clean完整构建作为交付。

## 最终安装与独立验收

完整 clean 源码 `97c8b5b09366501d2fbb75063d0af1547ba9eae3` 已 release 构建并安装 `/Applications/Velune.app`，仍为0.1 beta.1／schema7。第一轮中间构建未安装。已核对来源commit、clean manifest、全部helper当前字节、strict deep签名；原应用正常退出，新版未以真实配置运行。

root从安装Rust dylib重新生成Python绑定，安装包以下手工入口均通过：session-browser（全部启用实例、created/updated、disabled不读来源、schema7默认不重置）；session-loading（实际AppStore/Transport跨实例、完整cwd分组、时间排序、慢读取+迟到poll、加载中原生rename/delete接受、删除目标不返回、失败loaded保留）；session-management（Pi/Codex真改名删除／持久性／source scope）；multi-runtime（实际Codex/DSH新建、延迟首轮、open/continue创建时间、五次loopback）；pi-native-loop（草稿与工具／后续轮，六次loopback）；provider-configuration及runtime-interactions（配置与审批／busy保护）。全部临时HOME、合成原生会话、本机loopback，无真实凭据、会话、配置读取和真实模型请求。不运行或新增自动化测试。

开发方实现、静态、构建、安装与隔离验收完成。图形菜单呈现、分组及筛选的原生体验留给用户GUI验收。自定义label/section只是已厘清的未来组织记录方向，本轮没有标签CRUD；DSH当前ACP原生管理能力仍明确不可用。

## 2026-10-18：折叠后再展开显示“加载更多（0/N）”

用户反馈：项目组折叠后再展开，出现“加载（0/6）”，会话出现一会又消失。日志（只看操作名与耗时）显示每次切换都是保存偏好与一次整表 `list` 交替执行，各约 0.75 秒：保存浏览偏好被标为改变配置，应用 store 因此再读一次列表；先发出的折叠保存及这些读取的结果晚于展开到达，把折叠时的分组（该组 loaded 0）套到已展开的组上。修正：保存偏好按写入处理（结果本身是完整列表，经共享 store 同步到各工作区），不再触发整表刷新；列表工作区在保存在途时不从共享 store 采用分组与会话；先发出的保存成功时不应用其列表，最后一次保存失败时重读列表。验证：临时让核心列表慢 0.7 秒、临时钩子先折叠 1.2 秒后展开并每 0.1 秒采样该组：修正前展开后 0.8 秒为 0/6、0 行，修正后始终 6/6；每次切换只剩一次保存操作。

## 2026-10-18：手动排序组

用户要求会话列表的组可手动排序（[S37](../../docs/sources.md#s37)）。浏览偏好增加 `group_order`（组键列表，serde 默认空，旧配置照常读取）；core 在按最近活动排好组之后，若所示组中有任一出现在 `group_order` 中，就按它重排：置顶组在最前，未在顺序中的组（之后新出现的）随后并保持最近活动顺序，其余按用户顺序。项目合并文件夹时与折叠状态一样跟随改键，合并后去重，保留第一个文件夹的位置。Mac：拖动组标题（只在本进程内有效的自有类型）放到另一组标题上，向下拖放到其后、向上拖放到其前；右键提供上移、下移、移到最前、移到最后，已手动排序时再提供“按最近活动排列组”（只清除当前分组方式的顺序）。移动只替换当前显示的组所占的位置，被筛选掉的组和另一分组方式的键保持原位。搜索时不排序。

验证（临时 HOME、合成 Claude Code 会话、假 claude 二进制，经 Python 绑定）：按最近活动、手动顺序、混入另一分组方式的键、折叠的组保持位置、新组排在最前、重启后保持、合并项目保留首个位置且顺序随之改键、恢复按最近活动，共 9 项通过；移动算法单独 8 例通过。开发构建副本在隔离 HOME 中后台启动，截图确认侧栏按设定的 beta、gamma、alpha 排列（按最近活动应为 alpha 在前）。拖动与右键菜单的交互未在后台验证，留给用户 GUI 验收。
