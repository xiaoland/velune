# Android Agent 操作指引

本文件面向开发、排障和验收 Android 端的 Agent。平台实现与构建说明归 [README](README.md)，真实数据操作范围遵守[仓库授权边界](../../docs/development.md#隔离验证与敏感数据)。

## ADB 设备与文本输入

- 先用 `adb devices -l` 核对身份，每条命令显式指定 `-s <serial>`。USB 和无线记录可能属于同一手机；选在线传输。offline 时先定向 `reconnect` 并复核，避免重启整个 ADB server 影响其它设备。
- URL、邀请和令牌默认整段粘贴。`input text` 会生成按键事件，可能受输入法影响。先用公开文本验证通道，在应用实际字段中核对完整值，再填凭据；命令成功不足以证明文本一致。
- 已授权且有受控来源的凭据，先按本机指引和服务文档定位文件，由脚本读取、传输和填入；不要把可自动完成的工作转交用户重输或要求在聊天中发送令牌。
- 解密输出通过管道读入内存，再经 ADB stdin 传递；凭据不进入宿主命令行参数、shell 历史、日志、URL 或普通临时文件。设备 shell 需要解析时使用正确的 shell quoting，JSON 编码不代替转义。
- 先检查 `cmd clipboard` 是否实现。可用临时 `app_process` helper 调用当前系统的剪贴板接口，值只从 stdin 读取，标记 `android.content.extra.IS_SENSITIVE`，仅输出成功／失败。聚焦输入框，Ctrl+A 后用 `KEYCODE_PASTE` 整段替换，立即清除本次临时剪贴项。先用公开文本验证该系统与控件，隐藏 API 签名不作为跨版本保证。密码框检查只输出匹配或格式判断。

## 排障证据与验收

- 重启前先定位错误页面、触发操作并保留必要投影，避免丢失内存草稿。原草稿已不可用时，替换配置成功不能证明原输入的具体错误。
- release 不支持 `run-as`，也不启动 debug 本地控制入口；按安装身份选择诊断方式，保留日常数据与签名。
- UI 检查优先使用内存状态。设备支持时可用 `adb -s <serial> exec-out uiautomator dump /proc/self/fd/1` 流式读取 XML；真实现场只输出目标包的已知静态文案和必要判断。完整界面证据留给授权的合成环境。
- logcat 按当前 PID、时间和错误范围筛选；LGTM 核对时间、平台与实例。无日志不证明无错误，也不单独证明遥测关闭。
- PID 存活或 Activity 启动成功只证明启动。设置验收分别检查保存确认、离页重开、保存完成后的 force-stop／冷启动持久化、设置应用状态与设施可达；设施可达不代替获授权的实际业务或中转验收。
- 使用 `GalleryActivity` 切换 `--es screen <id>` 预览不同合成场景时必须带 `-S`（如 `adb -s <serial> shell am start -S -W -n app.velune.android/.GalleryActivity --es screen conversation`）；不带 `-S` 会复用旧 Activity 实例而停留在上一场景。新增或回归 UI 场景优先沉淀到 `app/src/debug/kotlin/app/velune/android/preview/` 的具名目录。
- 涉及输入区或底部面板的布局改动，真机验收须在约 360 dp 宽度下同时核对常规态与真实软键盘展开态，确认底部操作按钮尺寸与对比度、键盘贴合与无胶囊裁切。

## 临时资源恢复

目标控件不可见时先检查电源和锁屏状态。临时保持亮屏前记录 `stay_on_while_plugged_in` 原值，在 `finally` 中恢复；需要用户认证的锁屏由用户解锁。完成后清理本次 helper、设备／本机临时文件、剪贴项与临时端口映射，只撤销本次创建的资源。
