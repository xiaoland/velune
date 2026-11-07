# 数据迁移

[返回开发导航](../development.md)

## 破坏性变更与启动迁移

公开 beta 起，不得通过 hard-reset／hard-cutoff 丢弃已有用户数据。持久格式变化必须交付迁移、可恢复备份及结果核验；无法识别的旧格式、未来格式或损坏文件应保留并明确报错，不得当作新安装创建空状态覆盖。正文、附件、配置、组织、任务、设备身份与授权、布局和偏好均遵守这一约定；只有明确可重建且不含用户状态的缓存可以重建。原 Harness 会话与认证的处理边界保持不变。来源见 [S84](../sources/development.md#s84)。

用户已授权首次公开 beta 启动时自动备份并迁移。Mac 启动先同步打开本机 `Application` 并持有 `runtime.lock`，然后才读取外观、快捷键、SceneStorage 与窗口恢复点；若锁或核心打开失败，本次启动禁用这些 native 状态写入。其他本地宿主在其排他锁后、打开业务存储前运行内置 preflight。迁移只检查本宿主持有的数据，预检本次确需转换的文件，再对这些源创建权限受限的恢复副本，并用原子文件替换或 SQLite 事务完成转换。不会复制整个状态目录或重写已经是当前版本的数据。失败会保留源文件与已创建的恢复副本；已知升级转换应可重跑。未知、未来或损坏格式保留原字节并明确报告，不会以默认值替换。降级时旧版本必须拒绝自己不认识的格式；恢复需先退出应用，再从备份恢复对应文件或升级到支持该格式的版本。

迁移代码归各持久存储的 owner；应用启动入口仅协调锁、预检、备份和结果，不以产品版本代替数据 schema、不建立重复业务规则或任意长的多版本框架。公开 beta 后，持久字段或语义变化必须递增 owner 的 store schema，并在读取前拒绝不支持的版本；同版本内未知字段不写回丢弃。离线脚本仍可用于开发期数据整理和人工恢复，但不是普通用户首次 beta 升级的前置依赖，不能绕过锁或自动迁移入口。

已有 `scripts/migrations/` 与 `scripts/run-migrations.py` 是开发期显式整理工具和人工恢复辅助，不等同于 beta 的内置迁移实现。普通用户不需要安装 Python。每个历史转换仍须在 Rust/native 启动 owner 中有合成旧格式验收后，才算已纳入 beta 基线；脚本本身不能作为“升级受支持”的证据。

当前内置转换覆盖普通配置 schema 10→11、合并式 composer 输入 schema 1→2、composer 选择 schema 1→2（同时将已知新会话策略归属迁入实例设置）、Mac 输入区绝对宽度转比例，以及已知未版本化的定时任务表识别与版本基线登记。Mac 宽度按 1280pt 除算并钳制到 50%–100%。Android remote state 的 schema 3→4 保留联系人、pin 与控制授权，新增 profile grants 与 pending pairings 为空；精确旧字节先写到同目录的私有备份。已是当前格式或新安装不产生备份。较早 pre-beta serviceTier、Codex 实例/关联、早期 composer-drafts 与平台缓存通过已有显式迁移 runner 作为过渡格式进入此基线；应用识别后会保留原文件并显示可执行命令。runner 使用与 beta 源码相同版本的脚本；若旧 Codex CLI 不可用，脚本停止并要求提供已安装的 0.166 CLI。Android 应用私有目录不能由该 runner 直接读取，签名切换前的导出／导入步骤见[发行任务包](../../tasks/first-public-beta/packet.md)。

Composer 首次接入也在提交新目录前保存组合格式恢复点。共享目录或平台缓存写入中断后，重跑继续完成缓存转换并沿用原身份；更早脚本留下的无恢复点旧缓存保留本地选择，并以未知基线交给既有冲突恢复处理。

对应任务记录迁移的来源与目标格式、执行方式和验证结果；未完成的迁移明确保留为升级前的待办。真实数据与凭据操作仍遵守[隔离验证与敏感数据](../development.md#隔离验证与敏感数据)。本约定修订此前“不写迁移、直接清空旧普通配置”的规则，来源见 [S63](../sources/development.md#s63)。

## 旧按轮档位转模型变体

本节和下方脚本步骤记录较早 pre-beta 数据的显式过渡实现。旧 composer 选择中的 `serviceTier` 不再属于新格式；即使值为 `null`，严格读取也会拒绝。应用会保留原文件并提示在与发布版本匹配的源码目录退出 Velune 后运行 `python3 scripts/run-migrations.py --home <VELUNE_HOME>`；runner 按顺序执行 serviceTier 与后续 composer 转换。不得只运行 `composer-state.py` 或清空选择绕过。

脚本检查 `composer-inputs.json`、`composer-choices.json`、`turn-requests.json`，以及 Mac 窗口恢复点与 Windows composer 本机缓存。空档位只移除旧字段，正文、附件、身份、修订、模型、推理强度和权限保持原值。非空档位在明确选择本机 Codex 原生模型时转为 adapter 的模型变体 ID，并保留提供商身份；历史轮次同时更新显示模型名称。不读取 Harness 会话或认证文件。

隐式模型、managed 模型、其他／无法核实的运行时，以及互相冲突的模型变体都不能由脚本猜测；这些情况整批拒绝，不写入任何源文件。先在旧版中选择明确的原生模型，或人工将 managed 模型配置为携带等价请求参数的独立条目并调整相关选择，再重新预览。原文件以 `0600` 权限完整保存在 `before-service-tier-model-variants-*` 目录；写入逐文件原子替换，中断后可重跑。独立运行 `composer-state.py` 遇到尚未转换的旧档位也会先拒绝，避免重新编码选择时丢失档位。

## Codex 0.166 实例迁移

本次删除 `codex-0.159` 适配器，保留独立的 `codex-0.166` 类型；没有产品中的“升级”关系。该转换需要核实本机 CLI 版本，因此保留为显式过渡脚本；应用识别旧实例或关联段时提示 `python3 scripts/migrations/codex-166.py --home <VELUNE_HOME> --apply`，必要时按脚本说明指定 0.166 CLI。原文件保持直到脚本确认适用性并备份。

```sh
python3 scripts/migrations/codex-166.py --home <VELUNE_HOME>
python3 scripts/migrations/codex-166.py --home <VELUNE_HOME> --apply
# 可明确指定现成的 CLI；不安装或更新外部运行时：
python3 scripts/migrations/codex-166.py --home <VELUNE_HOME> --binary <Codex-0.166-绝对路径> --apply
```

脚本核实 CLI 为 0.166.x，保留实例 ID、HOME、名称及其余设置，将旧实例类型和所有旧关联段类型转换为新身份，包括已删除实例遗留的关联段，避免旧枚举阻碍读取。未指定 binary 时优先沿用已配置的 0.166 CLI，否则检查已安装的 ChatGPT／Codex 应用内 CLI 及 PATH；没有匹配版本时整批拒绝，原文件保持。SSH 实例需先在执行机器核实 CLI，再手动转换类型、binary和对应关联。实例与关联均完整私有备份，逐文件原子替换，中断后可重跑。不会修改原生会话、认证或 Harness 配置。

不要仅改实例类型而遗漏已有会话关联，也不要用清空数据代替迁移。

## 运行时新会话策略归属

`runtime-start-policy.py` 在既有 Codex 类型和 composer 内容迁移之后执行。它把 `composer-choices.json` schema 1 中每个实例的 `newConversation` 搬到 `generic-config.json` 的实例 `settings.newConversation`；普通配置仍为 schema 11。策略经共享核心的 typed codec 读写，与实例字段、执行位置／HOME 和 ACP launch 一次提交。`composer-choices.json` 改为 schema 2，只保留带 `runtimeType` 的 `lastChosen` 与原有历史意图；旧类型的上次选择不参与新类型默认，不清空会话目标的明确选择、正文或附件。

该脚本保留为显式过渡的实现参考；启动 preflight 对 schema 1 choices 会迁移策略，使用同一 JSON 备份集一起变更普通配置与 choices。重复／孤立的实例选择、未知类型或冲突策略整批拒绝。

## Mac 窗口恢复身份

`SavedWindowLayout.windowDevice` 与 `windowKind` 定义恢复点的执行设备和窗口类型。更早的恢复点只能凭原 scene UUID 读取。版本切换若失去 SceneStorage 身份，应在部署时显式确认旧窗口的设备归属并迁移恢复点，确保其进入对应设备及类型的恢复候选。

修复已有缺口时，先正常退出并取得状态目录排他锁，完整备份旧恢复点和当前 scene 恢复点。确认原窗口布局完整、会话来源属于目标执行设备后，为它标注设备和窗口类型，再以同一份布局恢复当前 scene 的 journal。保留原有分组、Tab UUID、顺序、焦点及输入身份，启动后核对应用重新保存的结构。来源不明的旧恢复点继续保留，交由明确映射处理。

## Composer 未手选默认

`target-intent.py` 在既有 schema 2 上清理 `choiceEdited=false` 记录中的旧自动 seed，让运行时、owner 与各字段重新继承实际目标事实。修改的目标 revision 加一，目录 revision 每批加一。脚本同时移出 Mac 恢复点和 Windows 缓存中的自动初始化待写记录，完整私有备份先于任何修改。正文、附件及绑定保留。`choiceEdited=true` 原样保留，不能可靠反推旧客户端曾随手选项一并固化了哪些默认值。

该清理转换尚未移植到启动 preflight；命中旧自动 seed 的恢复缓存会保留，不作猜测或写回。
