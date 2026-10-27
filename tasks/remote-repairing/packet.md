# 撤销授权后重新配对与本地 CLI

## 当前结果与剩余工作

2026-10-27 用户要求整理工作区并重新安装 Mac 及 ADB 连接的两台 Android，明确授权提交全部未提交成果，且不必重新验证改动。已有远控成果提交为 `636d5b4`，独立进程调查记录提交为 `9e8881d`；随后从干净工作区完成构建，Mac、Huawei 与 Redmi 均已覆盖安装并启动。本轮保留应用数据，不新增功能、不补跑功能回归。

2026-10-23 已通过 Mac 与 Redmi Turbo 3 的真实配对闭环，全部由 CLI 操作：生成新邀请、核对两端核对码、批准、首次连接；Mac 撤销后同一 Controller 进入 blocked/unauthorized；重新邀请并批准后自动恢复 connected，connection_epoch 从 1 增至 2。验证期间 Redmi 的 VPN 关闭。不能据此认定 VPN 是此前握手失败的根因，也未验证开启 VPN 后的结果。

临时诊断代码已从 endpoint.rs、pairing.rs 移除。最终版本已完成 Mac 与 Android 构建、安装 Mac／Redmi／Huawei；Mac 签名严格验证通过，安装的主程序及核心库 SHA256 与构建产物一致。最终版本再次通过撤销后重新配对：同一 Controller 从 blocked/unauthorized 自动恢复 connected，connection_epoch 从 2 增至 3。当时按用户要求未提交源码；本轮提交授权见上文。

## 授权与边界

用户要求修复撤销后重新配对、构建安装 Mac 和两台 Android 手机，后来明确要求 Agent 直接完成真实配对，并为 Velune 留下 CLI 入口，避免 computer use 与重复人工操作。允许生成 Mac 邀请、传入 Redmi、核对并批准、连接及撤销后重新配对。必要的邀请和设备标识只在内存使用，不输出或保存邀请秘密，不读取会话、其他凭据或支付数据。手机序列号：Redmi b45097d7，Huawei HYJ4C19524006587。

root 负责实现与真机验证，advisor 只读检查授权／恢复边界及 CLI 接入判断。用户指出非法输入 UI 验证 ROI 低；停止扩展此类检查，优先真实流程。

## 修复与 CLI

合成身份、禁用公共发现和 relay 的 loopback 回归复现了恢复缺陷：配对确认只更新联系人，已有 Controller 的 Blocked 状态没有恢复；Android 缓存的应用对象继续使用它。修改前，新批准后状态仍为 blocked/unauthorized、connection_epoch=1；修改后回归通过。

当前 attempt 首次确认时清除阻塞并递增连接意图版本，保持选中、前后台及主动暂停状态。保存新配对保留暂停值；所有响应先核对当前 attempt，旧响应不能覆盖新状态。契约更新在 [remote README](../../packages/remote/README.md)。

Android 添加设备支持扫码／手动输入；输入复用 decodeRemoteOffer/beginPairing，相机权限在显式扫描时申请。已构建安装两台手机。

CLI 位于 [scripts/velune](../../scripts/velune)，复用 bindings 的现有 ServiceOwner，不另建身份或执行核心。Mac 与 Android debug 打开原 endpoint 后启动私有 Unix socket，目录0700、socket0600。命令为 status、invite、begin、approve、revoke、connect；邀请经 stdin，批准必须匹配当前 attempt、peer、核对码。Mac modal 观察同一服务，CLI 批准后关闭，避免过期弹窗再次拒绝。使用说明在 [开发文档](../../docs/development.md#本地配对-cli)。

Android 客户端使用 adb shell -T、run-as 和 toybox nc -U；exec-out 经实测不能可靠转发 stdin，已替换。Redmi 支持此通道，Huawei Android9 没有 nc/netcat，因此当前脚本不能直接操作 Huawei CLI。Android release 不开启该入口，Windows 未接入。

## 证据与排查记录

核心 `cargo test --locked -p velune-remote pairing::tests` 通过：首次连接、撤销后拒绝、再次批准前拒绝、再次批准后同一 Controller 恢复并完成查询，旧 attempt 六种响应拒绝，暂停／未选中／后台意图及重复 Granted 行为。remote fmt、clippy 通过，仅有仓库既有 clippy.toml 规则不可达提示。bindings cargo check、fmt、Mac 构建、Android 构建及 lint 通过。CLI Python 语法及 Mac 运行中 status 自检通过，随后已完成上述真机 CLI 闭环。

此前真机获批前仍失败：21:00 Redmi 在 PairBegin RPC 前返回 QUIC crypto 305（TLS access_denied），被归类 Unauthorized；21:10:47 手机拨号与 Mac incoming:seen 对齐，30秒后手机超时、Mac handshake_failed，证明至少该次到达 Mac。临时日志仅记录固定阶段与错误码，没有邀请、地址或业务内容。未修改 TLS 或授权策略，原错误来源尚未被唯一解释。

最终重装后，首次显式 connect 返回过 closed，随后后台恢复到 connected；当时 TracerPid=0，没有调试器残留。未取得足够证据唯一定位该次取消的原因，不能描述为所有重启／网络变更场景已修复。后续撤销与重新配对闭环通过，再次显式 CLI connect 也返回 connected。

CLI 调试曾因 ADB stdin 问题无响应；之后抓线程栈留下的 lldb-server 暂停了应用。接回 Redmi 后确认 /proc 中 tracing stop 与对应 TracerPid，终止本次调试器、清理复制的调试器文件并重启应用，CLI 随即正常响应。没有取得 runtime 锁死证据，没有增加 worker 或改动网络调度。ADB forward 已无残留。

## 构建安装

Cargo 使用 `CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo`，避免 registry 符号链接造成 Askama 模板路径错误。Mac 使用仓库 build-macos.sh --build-only，沿用现有 Apple Development 签名；通过 install-macos.py 覆盖安装 /Applications/Velune.app。Android 使用 build-android.sh，adb install -r 保留两台手机身份及数据。当前无诊断最终构建日志在 /tmp/velune-cli-final-macos.log 与 /tmp/velune-cli-final-android.log。

2026-10-27 重装沿用上述构建与安装入口。Mac manifest 记录源码 `9e8881d4c1b3759b61a1de6bb71e943fbc93224b`、`dirty=false`，安装器完成严格签名验证；安装后的主程序和核心库 SHA256 与构建产物一致。旧实例未响应正常退出，按开发文档已有授权发送 SIGTERM 后退出；新安装的应用已启动。Android 构建及脚本内置 lint 通过，两台设备的 `adb install -r` 均返回 Success，`am start -W` 均返回 Status: ok。Redmi 在启动成功后断开 ADB，之后未进一步检查该设备。新日志为 `/tmp/velune-workspace-reinstall-macos.log` 与 `/tmp/velune-workspace-reinstall-android.log`。本轮仅确认构建、安装和启动，不复测配对或业务功能。
