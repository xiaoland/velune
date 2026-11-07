# 开发环境与重装

[返回开发导航](../development.md)

在当前 Mac 开发工作区使用 SVC CLI 18.3.0（配置 schema 5）。配置归仓库 [svc.json](../../svc.json)：`dev.resources` 声明资源的观察与动作，`run` 提供构建和重装的命令入口。CLI 仅用于开发；产品运行不依赖它。先用 `svc status` 核对配置和接入状态；`svc dev list` 只列声明，`svc dev status <资源>` 才实际探测，`svc dev act <资源> <动作>` 执行声明的动作。

| 资源 | 能力与启动依赖 |
| --- | --- |
| `macos-app` | `reinstall` 以 release 构建、正常退出旧版、安装并在后台启动。host scope 只协调日常安装；开发编译走独立 run 入口 |
| `lgtm` | sfp7 上已有的 Docker Compose 接收端，停止保留数据卷 |
| `telemetry` | sfp7→本机的 Grafana 30300 与 OTLP/gRPC 34317；先确保 `lgtm` 就绪 |
| `telemetry-windows` | 本机→Windows 的相同回环端口；先确保 `telemetry` 就绪 |
| `telemetry-android` | 唯一 USB Android 的 ADB reverse 34317；先确保 `telemetry` 就绪 |

```sh
svc status
svc dev list
svc run build-macos
# 在 Velune 外的终端或 SVC Dev 中执行，会退出正在使用的 Velune：
svc run reinstall-macos
# 与重装入口调用相同的动作：
svc dev act macos-app reinstall
svc dev status telemetry
svc dev act lgtm ensure
svc dev act telemetry ensure
svc dev act telemetry-windows ensure
svc dev act telemetry-android ensure
svc dev act telemetry-android stop
svc dev act telemetry-windows stop
```

开发默认dev、在途构建复用、配套输出与空间维护归[开发构建与缓存](builds.md)。

## Mac 重装

Mac 重装先调用既有构建脚本并验证新 bundle 签名，成功后才通过 AppKit 请求目标位置的旧版正常退出。未保存修改或运行中任务仍由应用自己的退出确认处理；取消退出或 30 秒内未退出时停止重装，保留旧版。安装器校验同 bundle ID 的运行实例（拒绝时列出 PID 与路径）、暂存复制、签名与失败回滚。重装在任何退出动作前验证来源为签名完整的 daily 用途产物，目的地可显式接受保留原身份的旧日常版。日常目录固定由 OS 账号确定为 `~/.velune`；继承的 `VELUNE_HOME` 如指向其它目录会先拒绝，避免迁移与实际启动使用不同目录。安装完成后、启动前对该状态目录执行 `scripts/run-migrations.py`：目录锁仍被占用或任一迁移失败时不启动新版，由用户按输出处理。成功后 `open -g` 在后台启动新版。此命令须由用户从 Velune 外部执行，Agent 配置和验证入口时不执行日常重装。`svc run package-macos` 只准备日常产物，不退出或安装。只重装已有 daily 产物可直接运行 `bash scripts/reinstall-macos.sh --skip-build`；隔离目标可加 `--destination /absolute/Velune.app`。

## Mac 开发实例

```sh
svc run build-macos
svc run start-macos-dev
svc run status-macos-dev
svc run stop-macos-dev
# 性能调试仍为开发用途：
svc run build-macos-release
svc run start-macos-dev-release
```

开发用途由签名 bundle 的 `VeluneExecutionContext` 声明，与 dev／release 优化独立。每个规范化 worktree 根路径有稳定 scope，开发 bundle ID为 `local.velune.development.<scope>`，进程名为 `VeluneDev`，显示名为“Velune 开发版”。日常版保留 `local.velune.prototype`／`Velune`。不同worktree的系统偏好、窗口恢复及通知因此分开；开发版只注册自己的链接scheme，不接管 `velune`／`agents`。

开发默认状态目录为 `~/.velune-dev/<scope>`，使用短路径以适应Unix socket长度；原生Agent目录在其中的 `harness-user` 下。应用入口在核心、遥测和偏好初始化前核对用途与目录：缺失／矛盾身份、日常目录及其symlink别名、其它scope或无归属的已有非空目录均拒绝。临时合成目录用启动器的 `--home` 准备 `.velune-development.json` 标记；不复制日常配置、登录或会话。开发检查／预览只能由开发用途产物执行。文件目录隔离不等于同一账号Keychain隔离；真实账户联调仍需任务级授权。

`start`通过LaunchServices后台打开当前worktree的运行副本，构建不会改写已加载的bundle。副本只在实例退出后更新，每scope保留一个；不复制编译缓存。当前实例与请求的目录、构建来源不一致时，工具拒绝猜测复用，也不会自动重启。`status`显示PID、出生时间、准确bundle／executable路径、scope与状态目录；`stop`重新核对当前进程身份后只请求正常Quit。取消、超时或歧义时停止，不追加SIGKILL、不删除lock、不按名称搜索和结束其它Velune。

同worktree多Agent可复用构建和明确的开发实例；需要独立UI时使用不同worktree／scope。普通GUI不要裸执行bundle内文件，也不使用 `killall Velune`／`pkill -f Velune`，或通过退出日常实例来释放开发状态目录。既有 `--check-*` 程序化入口可直接运行开发可执行文件，但须显式使用私有合成目录、保留本次子进程句柄且只回收该子进程，不将其当作GUI启动方式。运行回执是观测结果，不是对某个PID永久有效的退出权限。原生日常重装仍须用户在应用外显式执行；同UID的任意外部shell依然能发SIGKILL，开发工具不声称构成OS安全沙箱。

## 通道与本机覆盖

SSH 和 ADB 继续承担实际传输；[服务动作](../../scripts/dev_services.py)只把就绪和有界停止交给 SVC，不维护另一套进程注册表。SSH control socket 位于本机 `~/.local/state/velune/dev/`，不会因工作树切换重复开通道。停止一个通道不级联停止上游。上游恢复时复用仍存活的下游隧道，端口冲突不接管其他服务。SVC 回执提供执行 ID 和日志位置；也可用 `--json` 获取机器结果。`svc run --follow <执行ID>` 跟随某次 run，`svc run --inspect <执行ID> --json` 读取其状态；已结束的 run 再次显式调用会重新执行。

共享配置使用已有 SSH 别名 `sfp7-ws.localhost`、`win-ws.localhost`，Android 默认选择唯一 USB 设备。需要不同主机或指定设备时，在被 Git 忽略的 `svc.local.json` 中用 schema 5 稀疏覆盖相应资源的 `observations.ready.probe.argv`、`actions.ensure.argv` 和 `actions.stop.argv`；设备参数为 `--device <serial>`，ADB 路径可用 `--adb` 指定。切换物理目标前先停止旧目标，并修改 `host_key`，避免与另一设备共享协调身份。认证仍由 SSH 和远端私有文件管理，不写入这些声明。

开发构建的本机环境放在 `run.<entry>.env`；重装仍放在 `dev.resources.macos-app.actions.reinstall.env`。CARGO_HOME须使用缓存真实路径，签名配置仍只保存在本机。日常打包的本机工具链与签名覆盖归 `run.package-macos.env`。旧的 `macos-app.actions.build.env` 覆盖需显式搬到Mac构建入口；本次工作区已完成此项本机配置调整，不由应用自动迁移。

```json
{
  "schema_version": 5,
  "run": {
    "build-macos": {"env": {"VELUNE_CODESIGN_IDENTITY": "Apple Development: …", "CARGO_HOME": "/absolute/cargo-cache"}},
    "build-macos-release": {"env": {"VELUNE_CODESIGN_IDENTITY": "Apple Development: …", "CARGO_HOME": "/absolute/cargo-cache"}},
    "package-macos": {"env": {"VELUNE_CODESIGN_IDENTITY": "Apple Development: …", "CARGO_HOME": "/absolute/cargo-cache"}}
  },
  "dev": {"resources": {"macos-app": {"actions": {
    "reinstall": {"env": {"VELUNE_CODESIGN_IDENTITY": "Apple Development: …", "CARGO_HOME": "/absolute/cargo-cache"}}
  }}}}
}
```
