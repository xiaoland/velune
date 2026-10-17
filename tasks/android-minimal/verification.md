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

冻结 host 动态库 SHA-256 为 `b57fad02bc813288a6b020c80e48da7f175000ed7cb035602c2679377f09b29b`，Python bindings SHA-256 为 `195689ad23e46ca0a8d899115c162c723e90b9c4cae15ebeffbc8442d2f11770`。本轮 Android 平台初始化未改变业务 UniFFI 契约。

## 当前观察

- `--prepare-only` 已实际运行：创建并完成合成会话，退出时关闭唯一 ApplicationService。
- 独立 Wi-Fi iroh 宿主已启动，一次性邀请只写入权限 0600 的私有文件。
- Android APK、界面流程和实际 Direct／Relay 路径尚待真机执行；不得将上述宿主准备列为 Android 通过。
