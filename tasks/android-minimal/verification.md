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

最终配对使用的冻结 host 动态库 SHA-256 为 `3df87b7e3f87c543b102e5cf78eb90b1ef87cf665a0d050370cb56feea079142`，Python bindings SHA-256 为 `df8c3f22beb625743b735188dcd304ba8248e07a7bc7cce6f6e7392b7448bbaf`。首轮真实 Kotlin 编译发现 UniFFI 生成的 `close`／`List` 命名冲突，公开入口已改名为 `shutdown`／`ListConfiguration`；宿主已用匹配绑定重启。

Android arm64 native so SHA-256 为 `42a227f5be358d262fbcee2a82ad9a240d2a59f2b51da1170da998ab452299d9`。native owner 已完成目标平台 check／clippy／release，并确认 JNI 导出；实际设备装载另行观察。

首版 APK SHA-256 为 `6e1d2a6ff87f521669204d6cb06c82bd70a587feab0ac43dc2718705fa7b5a44`。解包确认只含 arm64-v8a native libs，并包含 Rust so 与 JNA `libjnidispatch.so`；APK 内 Rust so SHA 与上述产物一致。设备 package metadata 确认为 minSdk 28、targetSdk 36、primaryCpuAbi arm64-v8a、版本 `0.1 beta.1`。

## 当前观察

- `--prepare-only` 已实际运行：创建并完成合成会话，退出时关闭唯一 ApplicationService。
- 独立 Wi-Fi iroh 宿主已启动，一次性邀请只写入权限 0600 的私有文件。
- APK 已通过 `adb install -r` 安装，`am start -n app.velune.android/.MainActivity` 后进程存在，限定本 app PID 的 E 级日志未见错误。设备系统锁屏 `secure=true`，普通唤醒／上滑后仍未解锁；未读取通知、未截图锁屏、未尝试个人凭据。界面与业务结论必须等待用户本人解锁后取得。
- Android 界面流程和实际 Direct／Relay 路径尚待真机执行；不得将上述宿主准备或进程存在列为 Android 业务通过。

最终 APK 正在纳入 UniFFI Kotlin `android = true` 的 Android Cleaner 配置修复；以上首版 SHA／安装观察不作为修复后产物的装载或业务证据。最终 APK 需重新安装并记录 SHA。
