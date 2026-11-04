# 整机网络缓冲区诊断

[返回开发导航](../development.md)

遇到 `No buffer space available` 或所有应用无法联网时，在重启前运行 `python3 scripts/diagnose-network-macos.py`。输出默认保存在 `~/Library/Logs/Velune/network-时间.jsonl`，包含 mbuf／文件资源计数、按 PID 汇总的 TCP／UDP 数量、收发队列及累计流量／重传计数；不保存远端地址、请求内容、凭据或进程参数，不更改网络设置。队列长度不是累计流量，同一 PID 的相邻 `bytes_in_total`／`bytes_out_total` 差值才表示采样间收发量，进程退出重建后不跨身份比较。用 `--count 120 --interval 10` 可作约 20 分钟有界观察，Ctrl-C 可停止；每条记录立即写入，已有文件不覆盖。

内核缓冲区耗尽属于系统级现象；某进程等待分配只说明它受影响，不能单凭堆栈归因。需结合故障前后计数与进程退出后的变化判断。不要以增大系统缓冲区、强杀系统扩展或高负载复现代替诊断。

## 故障前后的独立记录

只在明确需要持续排障时安装：

```sh
python3 scripts/network-flight-recorder-macos.py install --interval 60 --traffic-interval 300
python3 scripts/network-flight-recorder-macos.py status
python3 scripts/network-flight-recorder-macos.py stop
python3 scripts/network-flight-recorder-macos.py uninstall
```

安装将两个脚本复制到 `~/Library/Application Support/Velune/NetworkDiagnostics`，以用户 LaunchAgent 独立运行；不依赖仓库路径或 Velune 进程，不连接远端。默认七天到期正常退出，异常退出才重启；stop 同时禁用下次登录启动，uninstall 保留记录。`records/` 逐条 fsync，文件 0600、目录 0700，最多 16×8MiB；观察期限不是历史保留期限，当前约 100KiB/条在 60 秒间隔下约保留 22 小时，实际以体积为准。

记录包含 boot、进程出生身份与父子链、已确认 Velune 后代重挂后的延续、全局 mbuf、TCP/UDP 状态与队列、接口计数及低频累计流量。当前 XNU ABI 还支持 kernel-control 逐 unit 及 TCP/UDP 按 last_pid 汇总的 socket 队列 mbuf 占用；不保存地址、端口、进程参数、请求或凭据。ABI 不匹配、读取失败、样本超时与缺口明确标记，不能读成零。系统进程可能仅有秒精度出生时间；同秒 PID 复用无法区分。socket last_pid 是最后操作进程，不保证独占所有权，须结合采样前后身份与队列变化。status 同时给出样本新鲜度与覆盖缺口，持锁不等于采集正常。

复发时先保留记录，再进行恢复：正常退出 Velune 后确认主进程及已识别后代是否真正结束，观察全局与 socket 队列变化；如用户选择停用／重开代理，核对系统扩展 PID 与出生时间是否真正变化，不能只以 GUI 重开为准。工具不自动杀进程或更改网络设置。队列集中增长可定位持有层；所有已观察队列较小仍不能排除 TCP 重组、IP 分片、驱动／Skywalk 等未观察内核路径，也不能排除先前触发条件。
