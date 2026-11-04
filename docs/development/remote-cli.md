# 本地配对 CLI

[返回开发导航](../development.md)

启动 Velune 后，在仓库运行 `scripts/velune` 操作该应用正在使用的远控服务；CLI 不另开设备身份或执行核心。Mac 默认连接 `$VELUNE_HOME/remote/cli/control.sock`（未设置时为 `~/.velune`）。Android debug 应用通过 `--android <adb序列号>` 和 `run-as` 访问同一接口，设备的 toybox 需支持 `nc -U`；release 包不启动此入口。监听仅使用私有目录中的 Unix socket，不开放 TCP 端口。

```sh
scripts/velune status
scripts/velune invite | scripts/velune --android <adb序列号> begin
scripts/velune --android <adb序列号> status
scripts/velune approve --attempt <本次attempt> --peer <控制端peer> --code <两端一致的八位核对码>
scripts/velune --android <adb序列号> connect --peer <被控端peer>
scripts/velune revoke --peer <控制端peer>
```

`invite` 与界面的“添加控制设备”相同，会开启控制入口并生成五分钟邀请；输出含邀请秘密，应直接传给 `begin`，不要保存到日志。其余命令返回 JSON，失败退出码非零。`status` 只返回远控状态，不读取会话正文。批准要求当前 attempt、预期 peer 和核对码同时匹配；CLI 与界面共用授权规则。应用退出时接口关闭；修改 `VELUNE_HOME` 或使用隔离实例时，CLI 必须连接对应实例的 socket。
