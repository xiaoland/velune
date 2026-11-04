# 开发遥测

[返回开发导航](../development.md)

普通启动与 Debug 构建默认不启用遥测。使用 `scripts/dev-telemetry.py launch` 显式启动隔离实例时，工具通过[开发环境](environment.md)中的 SVC 入口准备通道并检查 Grafana 与 OTLP 就绪，生成私有临时 profile 和 run manifest。它不修改日常偏好或全局 `OTEL_*`，也不重建／清空 LGTM。业务 home 与 OS home 使用本次运行的独立临时目录，避免长仓库路径超过 Mac 的 Unix socket 长度限制。接收端不可用时继续启动业务并报告未接入；端口冲突不停止无关进程。

先按[平台构建说明](platforms.md)生成产物，然后在仓库运行以下入口。桌面 `--app` 必须是已安装或已构建产物的绝对路径；Mac 指向 bundle 内可执行文件，Windows 指向 `Velune.exe`。移动端省略 `--app` 直接使用已安装应用；只有明确传入 APK／bundle 才执行覆盖安装。设备必须已启动；入口会重新启动指定应用，以便读取本次临时配置。

```sh
python3 scripts/dev-telemetry.py launch macos --app /absolute/path/Velune.app/Contents/MacOS/Velune
python3 scripts/dev-telemetry.py launch windows --app C:/absolute/path/Velune.exe
python3 scripts/dev-telemetry.py launch ios-simulator --device <Simulator UUID> --package app.velune.ios
python3 scripts/dev-telemetry.py launch android --device <ADB serial> --package app.velune.android.telemetry
```

Android 复用已安装的 debug 应用和合成配置。工具经 ADB reverse 把设备回环 34317 转发到开发机，只通过 `run-as` 的 stdin 写入 debug 应用私有 profile，不经过共享存储。该转发需要保留 ADB；手机与桌面的回环地址没有自动互换。Simulator 使用容器私有文件和 `SIMCTL_CHILD_*`，真机需要另行验证可达的受控入口。Windows 的原生 OpenSSH 不支持多路复用，工具复用显式准备的回环 30300／34317 转发并检查就绪；当前开发环境由 `svc dev act telemetry-windows ensure` 从 Mac 反向映射到 Windows，不开放 LGTM 到 LAN。

输出的 `manifestPath` 指向 `target/telemetry-development/<run-id>/run.json`。它含 run ID、应用路径、工作区当前提交（`workspaceRevision`）、进程／设备、状态路径、查询凭据文件位置与保留范围，不含认证值。该提交只标识启动工具所在工作区，不代表已安装应用的构建版本。复制的构建目录可以没有 Git，此字段为 null，不阻止应用启动。读取状态与限定本次运行的查询：

```sh
python3 scripts/dev-telemetry.py status /absolute/path/run.json
python3 scripts/dev-telemetry.py query /absolute/path/run.json traces
python3 scripts/dev-telemetry.py query /absolute/path/run.json logs
python3 scripts/dev-telemetry.py query /absolute/path/run.json metrics
```

状态 45 秒未刷新或 run ID 不匹配时视为过期；停止进程后不要把旧文件当作当前生效状态。状态文件缺失、容器路径不可读或内容无效时返回 `state_unavailable`，不能解释为关闭。SDK 的 `rpc_completed` 不证明三信号已经存入后台，必须分别查询并检查时间范围、实例、采样和结果限制。精确实时丢弃计数没有统一公开接口，未知值不能视为零；已知 SDK 丢弃汇总留在同目录 `sdk-diagnostics.json`，包含原修订并最多保留 64 条。Swift SDK 没有日志丢弃 hook。正常退出与移动端强制结束的证据边界不同。

查询默认选择 `velune.telemetry.schema.version="2"`。历史数据使用 `python3 scripts/dev-telemetry.py query <manifest> traces --schema-version legacy`，混合版本用 `--schema-version all`；logs 和 metrics 支持同一选项。旧属性 operation／operation_id／outcome 与 v2 的 velune.operation／velune.operation.id／velune.outcome 分开解释，不能把新查询无结果理解为旧记录消失。trace 可能跨版本，查看详细记录时按各自 resource 判断。legacy trace 的缺属性条件使用 Tempo 的 `= nil`，接收端需支持该语法。

语义与扩展范例归[遥测契约](../design/telemetry.md#扩展埋点与读取范例)。先限定 run、实例、时间和版本，再沿 trace、operation ID、RequestId 检查受理与执行。Loki 中点号属性会按其映射显示为下划线名称；查询应使用接收端实际字段，不解析日志 body。指标按操作层分别统计，不能将 UI、bindings 与 RPC 次数相加。Gateway 本地诊断的 transport_completed 仍表示传输结束，OTLP 的 outcome 则同时考虑 HTTP 结果；返回完整 503 在 OTLP 中是 failure。
