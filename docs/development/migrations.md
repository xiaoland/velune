# 手动迁移

[返回开发导航](../development.md)

## 破坏性变更与手动迁移

允许 hard-reset／hard-cutoff，但涉及已有数据或配置时，必须在切换到新格式的版本前完成显式手动迁移并核验结果，不能用“旧数据会被重置”代替迁移交付。原 Harness 会话与认证的处理边界保持不变。

迁移可以由 Agent 手工完成，也可以编写并显式执行一次性脚本。用户已[持续授权必要的数据迁移](../sources/development.md#s79)，Agent 应在修复或升级范围内完成，不逐次请求确认；执行前核对目标与排他锁，保留可恢复备份，执行后验证数据完整性。脚本自动处理转换仍属于手动发起的迁移；应用运行时不自动检测并转换旧格式，不引入长期维护的自动迁移框架、兼容层或多版本迁移链。

一次性迁移脚本归 `scripts/migrations/`，文件名顺序即执行顺序，接口统一为 `--home <VELUNE_HOME>`（必填）与 `--apply`（写入，缺省为预览）。脚本自行获取 `runtime.lock` 排他锁，不改动正在运行的核心；自检适用性并幂等，无事可迁移时以 0 退出，前置条件不满足（如需人工核对的实例）以非 0 退出并说明手工步骤。`scripts/run-migrations.py --home <dir>` 先探测目录锁占用并列出占用进程，再顺序执行全部迁移，任一失败即中止。Mac 重装在安装后、启动新版前对本机状态目录调用该入口（见[开发环境](environment.md#mac-重装)）；这是用户显式发起重装的一部分，不属于应用运行时自动迁移。

当前显式升级步骤包含普通配置 schema 10→11 的查询凭据类型转换、[旧按轮档位转模型变体](#旧按轮档位转模型变体)、旧 composer 输入接入及内容／目标选择拆分、Codex 实例类型调整、[运行时新会话策略归属](#运行时新会话策略归属)和 Mac 输入区宽度比例转换。宽度以 1280pt 为固定参考，将旧值除以 1280 并钳制到 50%–100%；保留原外观文件备份。完整 runner 在各步拒绝不认识的数据后停止，不能以直接启动新版绕过。

Composer 首次接入也在提交新目录前保存组合格式恢复点。共享目录或平台缓存写入中断后，重跑继续完成缓存转换并沿用原身份；更早脚本留下的无恢复点旧缓存保留本地选择，并以未知基线交给既有冲突恢复处理。

对应任务记录迁移的来源与目标格式、执行方式和验证结果；未完成的迁移明确保留为升级前的待办。真实数据与凭据操作仍遵守[隔离验证与敏感数据](../development.md#隔离验证与敏感数据)。本约定修订此前“不写迁移、直接清空旧普通配置”的规则，来源见 [S63](../sources/development.md#s63)。

## 旧按轮档位转模型变体

速度改为模型目录中的独立条目后，旧 composer 选择中的 `serviceTier` 不再属于新格式；即使值为 `null`，严格读取也会拒绝。切换前退出使用该状态目录的所有 Velune 进程，运行 `python3 scripts/migrations/01-service-tier-model-variants.py --home <VELUNE_HOME>` 预览，再加 `--apply` 写入；随后执行 `composer-state.py` 完成输入格式拆分。完整 runner 已按此顺序执行这两步。

脚本检查 `composer-inputs.json`、`composer-choices.json`、`turn-requests.json`，以及 Mac 窗口恢复点与 Windows composer 本机缓存。空档位只移除旧字段，正文、附件、身份、修订、模型、推理强度和权限保持原值。非空档位在明确选择本机 Codex 原生模型时转为 adapter 的模型变体 ID，并保留提供商身份；历史轮次同时更新显示模型名称。不读取 Harness 会话或认证文件。

隐式模型、managed 模型、其他／无法核实的运行时，以及互相冲突的模型变体都不能由脚本猜测；这些情况整批拒绝，不写入任何源文件。先在旧版中选择明确的原生模型，或人工将 managed 模型配置为携带等价请求参数的独立条目并调整相关选择，再重新预览。原文件以 `0600` 权限完整保存在 `before-service-tier-model-variants-*` 目录；写入逐文件原子替换，中断后可重跑。独立运行 `composer-state.py` 遇到尚未转换的旧档位也会先拒绝，避免重新编码选择时丢失档位。

## Codex 0.166 实例迁移

本次删除 `codex-0.159` 适配器，保留独立的 `codex-0.166` 类型；没有产品中的“升级”关系。先准备包含新代码的构建，正常退出使用该目录的应用后执行：

```sh
python3 scripts/migrations/codex-166.py --home <VELUNE_HOME>
python3 scripts/migrations/codex-166.py --home <VELUNE_HOME> --apply
# 可明确指定现成的 CLI；不安装或更新外部运行时：
python3 scripts/migrations/codex-166.py --home <VELUNE_HOME> --binary <Codex-0.166-绝对路径> --apply
```

脚本核实 CLI 为 0.166.x，保留实例 ID、HOME、名称及其余设置，将旧实例类型和所有旧关联段类型转换为新身份，包括已删除实例遗留的关联段，避免旧枚举阻碍读取。未指定 binary 时优先沿用已配置的 0.166 CLI，否则检查已安装的 ChatGPT／Codex 应用内 CLI 及 PATH；没有匹配版本时整批拒绝，原文件保持。SSH 实例需先在执行机器核实 CLI，再手动转换类型、binary和对应关联。实例与关联均完整私有备份，逐文件原子替换，中断后可重跑。不会修改原生会话、认证或 Harness 配置。

完整 runner 包含本步骤，日常重装会在启动前执行；已经安装166类型但仍有旧关联时也能修复。不要仅改实例类型而遗漏已有会话关联，也不要用清空数据代替迁移。

## 运行时新会话策略归属

`runtime-start-policy.py` 在既有 Codex 类型和 composer 内容迁移之后执行。它把 `composer-choices.json` schema 1 中每个实例的 `newConversation` 搬到 `generic-config.json` 的实例 `settings.newConversation`；普通配置仍为 schema 11。策略经共享核心的 typed codec 读写，与实例字段、执行位置／HOME 和 ACP launch 一次提交。`composer-choices.json` 改为 schema 2，只保留带 `runtimeType` 的 `lastChosen` 与原有历史意图；旧类型的上次选择不参与新类型默认，不清空会话目标的明确选择、正文或附件。

退出占用状态目录的 Velune 后，可先用 `python3 scripts/migrations/runtime-start-policy.py --home <VELUNE_HOME>` 预览，再加 `--apply` 写入；完整 runner 已包含此步骤。原文件以 `0600` 权限保存在 `before-runtime-start-policy-*`。先提交普通配置，再提交 composer；两次提交之间中断时，旧 composer 仍是恢复源，重跑仅接受与源相同的已写策略，不覆盖原备份。重复／孤立的实例选择、未知类型或冲突策略整批拒绝，需人工核对。应用只读取新 composer schema，不自动转换或继续双写旧归属。

## Mac 窗口恢复身份

`SavedWindowLayout.windowDevice` 与 `windowKind` 定义恢复点的执行设备和窗口类型。更早的恢复点只能凭原 scene UUID 读取。版本切换若失去 SceneStorage 身份，应在部署时显式确认旧窗口的设备归属并迁移恢复点，确保其进入对应设备及类型的恢复候选。

修复已有缺口时，先正常退出并取得状态目录排他锁，完整备份旧恢复点和当前 scene 恢复点。确认原窗口布局完整、会话来源属于目标执行设备后，为它标注设备和窗口类型，再以同一份布局恢复当前 scene 的 journal。保留原有分组、Tab UUID、顺序、焦点及输入身份，启动后核对应用重新保存的结构。来源不明的旧恢复点继续保留，交由明确映射处理。

## Composer 未手选默认

`target-intent.py` 在既有 schema 2 上清理 `choiceEdited=false` 记录中的旧自动 seed，让运行时、owner 与各字段重新继承实际目标事实。修改的目标 revision 加一，目录 revision 每批加一。脚本同时移出 Mac 恢复点和 Windows 缓存中的自动初始化待写记录，完整私有备份先于任何修改。正文、附件及绑定保留。`choiceEdited=true` 原样保留，不能可靠反推旧客户端曾随手选项一并固化了哪些默认值。

退出占用状态目录的应用后，用 `python3 scripts/migrations/target-intent.py --home <VELUNE_HOME>` 预览，加 `--apply` 写入；完整迁移入口按文件名顺序执行本步骤，重跑继续尚未完成的文件。应用不在启动时自动猜测旧选择。
