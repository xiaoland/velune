# Android 最小端隔离验收

## 当前结论

2026-10-17，用户本人解锁后进行过 Android 真机探索，随后要求暂停实现与验收并重新设计。Android 身份发布修复已提交 `0004d53`；现仅继续设计，设备操作没有恢复。没有将安装、进程存在或临时原生 probe 当作业务通过。

| 边界 | 当前证据 | 结论 |
| --- | --- | --- |
| Android 构建与打包 | app owner 的 assembleDebug／lintDebug 通过；APK 内 so SHA 与修复产物一致 | 通过构建边界 |
| 真机安装／启动 | `adb install -r` 成功，本 app 进程存在，PID 限定日志没有 AndroidRuntime／JNA／UniFFI 崩溃 | 通过安装与进程启动边界 |
| 身份初始化 | 修复 hardlink 后已产生 0600／168 字节 identity；诊断包的新鲜 dump／截图确认错误消失；最终无诊断包待冷启重验 | 修复已落地，最终界面重验待完成 |
| Wi-Fi 配对与路径 | 实际 UI 尝试尚未成功；暂停前未完成输入注入与产品行为的区分，未使用 adb reverse／forward | 尚未配对，原因未定 |
| 会话／消息／停止／审批／问答 | 已准备真实 ApplicationService＋合成 ACP／Claude 协议宿主，尚未通过 Android UI 操作 | 未验证 |
| 后台／断线继续、重连、原请求查询、撤销 | 尚未到达业务阶段 | 未验证 |
| Mac 设置入口 | `d3b039d` 隔离 clone 的真实设置启停／邀请／私有权限通过，详见下文 | Mac 本地入口通过；Mac→手机配对待验证 |

暂停时安装 APK SHA-256 为 `33cb77b3827b4ce870acfc284180fbb605b2203ca08fc72d2c46bcd309eebee8`，包含默认隐藏／可显式显示邀请及禁用自动纠正，不包含临时 native 诊断。其 so SHA-256 为 `2529e9142589abddf980c5c9da44876fcef8d369513337a827b7efb0cd32ba00`，与首次修复版字节完全一致。assemble／lint 通过，安装保留 app data；完整冷启、空数据首次创建与业务验收尚未完成。诊断 APK `ecab486` 仅用于先前定位，不能当成交付包或完整业务证据。

宿主使用匹配 main 集成契约的冻结动态库 SHA-256 `e84fc5b8fc43782e1109924f8f4c232a9b50bd185334b98c16404e8724917a25`，Python bindings SHA-256 `f07a5c7cbba659feabb25184c179b7f089644e7b0cb7f32413bd51fc033b90e4`。

## 环境与验收判据

设备为 API 28／arm64-v8a，屏幕 1080×2340、480 dpi，开发机与设备同 Wi-Fi。只操作本任务的 `app.velune.android`、公开设备能力和合成资料，不读取其他应用、账户、通知或个人文件。package metadata 确认为 minSdk 28、targetSdk 36、primaryCpuAbi arm64-v8a、版本 `0.1 beta.1`。

验收必须经过真实 Compose 界面、UniFFI、iroh 与执行端唯一 ApplicationService，执行合成协议 peer。网络回声、Python 直接业务调用或构建成功不能代替界面验收。邀请只存任务私有临时文件，不进入日志、截图或 Git；不调用真实模型或读取既有 Harness 配置／认证／会话。

`scripts/manual-android-remote.py` 显式创建临时 HOME、合成运行时和已完成的合成会话。`--prepare-only` 关闭唯一 application owner 后将配置根交给隔离 Mac app；前台宿主使用另一目录，避免共开配置根。

```sh
python3 scripts/manual-android-remote.py \
  --root /tmp/velune-android-final-host-fixture \
  --library "$PWD/target/remote-control-bindings/libvelune_bindings.dylib" \
  --bindings "$PWD/target/remote-control-bindings" \
  --resources "$PWD/target/runtime-support"
```

宿主显式采用 DevelopmentPublic relay／discovery 和 `0.0.0.0:0`，不代表产品已部署公共服务。Beta 的普通消息得到 `ACP_ANSWER`，`PERMIT` 请求真实 ACP 审批，`CANCEL` 持续执行至取消，`GATE` 等待私有 `hold` 文件释放。新增 Synthetic questions 运行时复用已有 Claude 协议 fixture，真实审批后要求问题回答；它不调用供应商服务。

## 身份存储失败、实验与修复判据

用户解锁后，真实欢迎页首次显示“无法读取或安全保存设备身份”。当时 `no_backup/remote` 已为 0700 且为空。按 native owner 指定，在本 app 私有目录下创建独立合成 `.storage-probe`：固定内容的 0600 marker 创建成功，同目录 `ln` 发布返回 `Permission denied`，link 不存在。仅清理本次 probe，未读 identity、未改现有权限。原 Rust 身份创建依赖 hard_link，Android 沙箱拒绝该边界。

随后检查临时原生 probe 源码，以本 app UID 对独立 `.rename-probe` 目录实测：`renameat2(RENAME_NOREPLACE)` 在目标不存在时发布成功；目标已存在时返回 `AlreadyExists` 且原合成内容保持不变；目录 fsync 成功。扩展 probe 用两个已同步的 4096 字节 A／B marker 并发争同一目标，结果恰一成功、另一 `AlreadyExists`，重开目标完整等于获胜来源内容。probe 只含固定合成 marker，完成后清理目录与可执行副本。

native owner 据此采用 Android 专属 rustix NOREPLACE 发布，不使用会覆盖目标的普通 rename fallback。修复 APK 覆盖安装后，identity 已发布为 0600／168 字节。曾报告“修复后重开仍失败”，随后发现该证据无效：uiautomator 在 busy 状态可以退出 0 而不更新旧 XML，初始临时 driver 未删除旧 XML、也未验证成功输出。driver 已改为每次先删除旧文件并要求明确 dump 成功；busy 记为未知，使用当前 screencap 交叉确认。新鲜 dump／截图实际确认诊断包身份错误已经消失，不能将旧 XML 当作剩余身份 bug。诊断撤除后的 so 精确等于首次修复版，最终无诊断 APK 仍需冷启、重新创建身份和配对实证。run-as probe 本身只支持其观测边界，不能代替真实 app 域的最终证据。

## Mac 本地入口与集成回归

主执行者在 `d3b039d` clean release clone、全新隔离配置根上完成真实 Mac 设置操作：初始未预建 remote，经设置启用显示“已启用”，独立 AX 停止按钮可达；生成五分钟有效一次性邀请，拷贝入口可见但未复制邀请码；停止后回到未启用且清除邀请。remote 目录／identity 权限分别为 0700／0600。较早的 `235b62b` clone 还在停止入口后经真实聊天界面发送 `MAC_AFTER_ENDPOINT_STOP` 并收到 `ACP_ANSWER`，证明停止入口不关闭业务核心；此项证据只归 `235b62b`。

`d3b039d` 安装前仅检查公开 build manifest，原安装 `3f01f9b`、dirty=false，核实它是构建提交祖先以避免回退 main 能力。正常退出已确认的 Velune 本体后原子安装，codesign deep／strict 通过；安装 manifest 为 `d3b039d`、dirty=false、版本 `0.1 beta.1`，未启动真实 app。安装 executable SHA-256 为 `2457b9b864532716d77d2fdfa1cf5e9978e480d375405a8d307233cc08a5fd6f`，dylib SHA-256 为 `391ba580dec7d4b09c785b78e27a068230cb62ea112b2c0b14d78f4151bb8886`，均与构建产物相同。后续 Android 修复不沿用此清单声称新源码已安装。

主执行者以 main 集成后的 host freeze、临时伪 bundle 和隔离 HOME 显式运行 `manual-provider-configuration.py`，原判据全部通过：合成 API key、私有文件 0600、摘要／日志无秘密、保存／重开／模板和 hard-cutoff。全 workspace locked check、严格 clippy、fmt 通过；没有新增或运行自动化测试。

## 历史观察与剩余范围

初轮设备处于 secure keyguard，曾仅完成安装与进程观察；用户此后已本人解锁，该前提已经解除。main 集成版旧 APK `3eacf0e86be715cd7dd0744d14978cc3b68c51dab64941fa65c2083c3deb75d4` 的安装／进程日志证据不能用于覆盖当前身份失败。

暂停时手机尚未配对成功、未提交合成业务操作。独立合成宿主已正常关闭 endpoint／ApplicationService，隔离 Mac 实例已退出；未清手机数据。后续恢复验收时按新方案核对实际状态，不沿用旧邀请粘贴流程的未完成观察。跨公网、真实切网、移动后台长期行为及真实模型体验未验证；同 Wi-Fi 不能证明公网路径。
