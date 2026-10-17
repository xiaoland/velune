# Android 最小端隔离验收

## 环境与判据

2026-10-17，使用 adb 已连接的 API 28／arm64-v8a 设备，屏幕 1080×2340、480 dpi。只操作本任务的 `app.velune.android`，不读取其他应用、账户或个人文件。设备与开发机同 Wi-Fi，未设置 adb reverse／forward；该条件只支持局域网路径结论。

验收必须经过真实 Android Compose 界面、UniFFI、iroh 与执行端唯一 ApplicationService，执行合成 ACP peer。网络回声、直接调用 Python 远控业务接口或构建成功不能代替界面验收。邀请属于一次性配对凭据，只保存在任务私有临时文件，不进入日志、截图或 Git。

## 隔离执行资料

`scripts/manual-android-remote.py` 复用已有合成 ACP 协议 peer，显式创建临时 HOME、两个合成运行时和一条已完成的合成会话。`--prepare-only` 关闭 application owner 后将同一配置根交给隔离 Mac app；前台宿主模式使用独立目录，避免两个 owner 共开配置根。

```sh
python3 scripts/manual-android-remote.py \
  --root /tmp/velune-android-host-fixture \
  --library "$PWD/target/remote-control-bindings/libvelune_bindings.dylib" \
  --bindings "$PWD/target/remote-control-bindings" \
  --resources "$PWD/target/runtime-support"
```

宿主显式采用 DevelopmentPublic relay／discovery 和 `0.0.0.0:0`；不代表产品已部署公共服务。普通消息得到合成 `ACP_ANSWER`，`PERMIT` 请求真实 ACP 审批，`CANCEL` 持续执行至取消，`GATE` 在私有临时 `hold` 文件存在时等待，释放后完成。开发宿主不请求真实模型或读取既有 Harness 文件。

Cleaner 修复阶段预备宿主的冻结动态库 SHA-256 为 `3df87b7e3f87c543b102e5cf78eb90b1ef87cf665a0d050370cb56feea079142`，Python bindings SHA-256 为 `df8c3f22beb625743b735188dcd304ba8248e07a7bc7cce6f6e7392b7448bbaf`。首轮真实 Kotlin 编译发现 UniFFI 生成的 `close`／`List` 命名冲突，公开入口已改名为 `shutdown`／`ListConfiguration`；宿主已用匹配绑定重启。

Android arm64 native so SHA-256 为 `42a227f5be358d262fbcee2a82ad9a240d2a59f2b51da1170da998ab452299d9`。native owner 已完成目标平台 check／clippy／release，并确认 JNI 导出；实际设备装载另行观察。

首版 APK SHA-256 为 `6e1d2a6ff87f521669204d6cb06c82bd70a587feab0ac43dc2718705fa7b5a44`。解包确认只含 arm64-v8a native libs，并包含 Rust so 与 JNA `libjnidispatch.so`；APK 内 Rust so SHA 与上述产物一致。设备 package metadata 确认为 minSdk 28、targetSdk 36、primaryCpuAbi arm64-v8a、版本 `0.1 beta.1`。

## 当前观察

- `--prepare-only` 已实际运行：创建并完成合成会话，退出时关闭唯一 ApplicationService。
- 独立 Wi-Fi iroh 宿主已启动，一次性邀请只写入权限 0600 的私有文件。
- APK 已通过 `adb install -r` 安装，`am start -n app.velune.android/.MainActivity` 后进程存在，限定本 app PID 的 E 级日志未见错误。设备系统锁屏 `secure=true`，普通唤醒／上滑后仍未解锁；未读取通知、未截图锁屏、未尝试个人凭据。界面与业务结论必须等待用户本人解锁后取得。
- Android 界面流程和实际 Direct／Relay 路径尚待真机执行；不得将上述宿主准备或进程存在列为 Android 业务通过。

首版之后已纳入 UniFFI Kotlin `android = true` 的 Android Cleaner 配置修复；首版 SHA／安装观察不作为修复后产物的装载或业务证据。修复版的重新安装观察见下文。

## Cleaner 修复版的安装观察

`android = true` 修复后的 APK SHA-256 为 `51667f5ac72e7174eb3e197bbe4aeb307a6ff2b0eb83339037ea9fa7c68e61e5`，已重新执行 `adb install -r` 成功，仅停止本 app 旧进程后重新启动，进程 PID 19992。限定该 PID 的 E 级日志只有厂商 AwareLog／MemoryLeakMonitor 缺少文件消息，没有 AndroidRuntime、JNA 或 UniFFI 错误。这仅支持安装／进程启动观察，不证明页面、配对或完整 JNI 业务调用已成功。

设备公共锁屏状态仍为 `secure=true`、`mIsShowing=true`；UI 验收需要用户本人解锁。未执行个人认证、修改锁屏设置或读取通知。

后续正在整合 main 的 ProviderTemplate 新契约，需重建最终 APK／host 后重新记录 SHA 和观察。上述证据只归其对应产物，不外推到后续版本。独立 LAN 宿主已正常停止，合成目录保留；该阶段隔离 Mac 的配置根由主执行者启动的 Mac app 独占，随后已按下文记录正常退出。

## Mac 产品入口的版本限定观察

主执行者在 main 集成前的源码 `235b62b` 上，通过隔离 Mac bundle 与全新的合成配置根完成以下人工操作；该证据不能外推为后续 main 合并产物的通过：

- 配置根初始无 remote 目录，经 Settings → 远程接入 → 启用成功；文件系统观察 remote 目录权限 0700、identity 文件 0600。
- 停止按钮以独立 AX button 可达；生成五分钟有效的一次性邀请成功。邀请未进入日志、文档或提交。
- 点击停止后回到未启用状态，界面清除邀请；关闭 Settings 后通过真实 Mac 聊天界面发送合成 `MAC_AFTER_ENDPOINT_STOP`，观察用户行及 `ACP_ANSWER`，证明停止远控入口未关闭唯一业务核心。
- 通过 Cmd-Q 正常退出隔离 Mac。当前没有临时 Mac application owner 持有合成配置；后续 main 合并产物需以新 fixture 重做对应观察。

## main 集成后的最终产物

源码 `d3b039d` 整合固定 main 后重新生成、构建的 APK SHA-256 为 `3eacf0e86be715cd7dd0744d14978cc3b68c51dab64941fa65c2083c3deb75d4`，APK 内 Rust so SHA-256 为 `1dd2711ab094446d1a71cb9566a69b37ab3da14b2ad71aa4be15ceb082f66a3c`。已重新 `adb install -r` 成功，只停止本 app 旧进程后启动，PID 20964。限定此 PID 的 E 级日志仅见 Huawei AwareLog／MemoryLeakMonitor 缺文件消息，没有 AndroidRuntime、JNA 或 UniFFI 错误。该证据仍只支持安装／进程启动，未将锁屏后的运行情况等同于业务调用成功。

匹配 host 动态库 SHA-256 为 `e84fc5b8fc43782e1109924f8f4c232a9b50bd185334b98c16404e8724917a25`，Python bindings SHA-256 为 `f07a5c7cbba659feabb25184c179b7f089644e7b0cb7f32413bd51fc033b90e4`。用此绑定创建全新隔离 Mac fixture，实际完成合成会话及回复后关闭 owner，再由主执行者启动最终 Mac clone 独占该配置根。独立 LAN 宿主使用另一个新 fixture，不与 Mac 共开。

主执行者另以此 host freeze、临时伪 bundle 和隔离 HOME 显式运行 `manual-provider-configuration.py`，原判据全部通过：合成 API key、私有文件 0600、摘要／日志不含秘密、保存／重开／模板以及 hard-cutoff。未调用真实模型或读取真实配置。该观察归属 `d3b039d` 的集成回归，不替代 Android 界面验收。

设备仍安全锁屏，配对／Direct 或 Relay／会话／消息／停止／审批／前后台接续／未决请求查询的 Android UI 证据均未取得。最终 Mac clone 的入口回归另行记录；公共网络与真实模型体验由用户验收。

## 最终 Mac 入口回归与安装

主执行者以 `d3b039d` 的 clean release clone、全新 final-mac fixture 进行真实 Mac 操作：配置根未预建 remote，经设置启用后显示“已启用”，独立 AX 停止按钮可达；生成五分钟有效的一次性邀请，拷贝入口可见但未复制邀请码；点击停止后回到未启用且邀请清除。实际目录／身份权限分别为 0700／0600，最后通过 Cmd-Q 正常退出隔离实例。

安装前仅检查公开 build manifest，原安装为 `3f01f9b`、`dirty=false`，核实它是 `d3b039d` 的祖先，避免覆盖丢失 main 已有能力。正常退出已确认的 Velune 本体后完成原子安装，`codesign --deep --strict` 检查通过；安装 manifest 为 `d3b039d`、`dirty=false`、版本 `0.1 beta.1`，未启动真实 app。已安装 executable SHA-256 为 `2457b9b864532716d77d2fdfa1cf5e9978e480d375405a8d307233cc08a5fd6f`，dylib SHA-256 为 `391ba580dec7d4b09c785b78e27a068230cb62ea112b2c0b14d78f4151bb8886`，均与构建产物相同。

主执行者完成全 workspace 的 locked check、严格 clippy 和 fmt；没有新增或执行自动化测试。

## 交付状态与未完成前提

最终 Android APK 已安装，未进入配对或业务界面；没有合成设备记录或未决操作需要清理，也没有真实用户数据访问。独立 final-host 已经 SIGINT 正常退出并关闭 endpoint／ApplicationService，合成资料保留，当前没有验收宿主运行。

唯一阻止 Android 真机界面验收的当前前提是用户本人手动解锁设备并保持亮屏。解锁后仍需实际完成邀请配对、确认 Direct／Relay 路径、会话创建与发送／停止／审批、断线与后台接续、可信重连及未决请求查询；这些项目当前均未取得证据，未宣称通过。跨公网、真实切网、移动后台长期行为及真实模型体验也未验证。
