# sfp7 开发可观测性接收端

2026-10-26 已部署并验证。此文记录本开发环境的实际入口、操作与证据；客户端的目标行为见[技术方案](technical-design.md)和[验收设计](acceptance.md)。四端接入与实际结果归[验收记录](verification.md)。

## 选择依据

本次保留 `grafana/otel-lgtm`。sfp7 具有 8 个逻辑 CPU、约 15.2 GiB 内存，部署前可用约 12.9 GiB、磁盘可用约 167 GiB。LGTM 已装配 Collector、Tempo、Loki、Prometheus 和 Grafana，提供 agent 可直接查询的三信号接口；其[官方定位](https://grafana.com/docs/opentelemetry/docker-lgtm/)是开发、演示与测试，适合本次用途，不作为生产托管方案。

轻量替代中，OpenObserve 的单进程形态最值得保留。[官方 OTLP 文档](https://openobserve.ai/docs/ingestion/logs/otlp/)支持三信号接入；[Hacker News 2023 讨论](https://news.ycombinator.com/item?id=36280028)有家用部署内存不到 100 MB 的反馈，[Reddit 2024 讨论](https://www.reddit.com/r/selfhosted/comments/1f3vyw6/openobserve_is_it_good/)则有约 150 GiB 日志测试占用 1.5–4 GiB 的反馈。这些是不同版本和工作负载的个人报告，不能当作本项目三信号的资源承诺。Victoria 系列同样值得关注，但[VictoriaTraces](https://docs.victoriametrics.com/victoriatraces/)配合 metrics、logs 和查询界面需要自行组合多个组件，本轮没有实测。

LGTM 当前短测占用低于设定预算，因此本轮不再维护第二套后台。若实际开发负载越过预算，再用相同合成负载比较 OpenObserve，不能拿不等价的社区数字直接替换。

## 部署与访问

| 项目 | 实际值 |
| --- | --- |
| SSH | `sfp7-ws.localhost`，当前指向 `yyh@172.16.249.6`，主机名 `surface-yyh` |
| 部署目录 | sfp7 上的 `/home/yyh/.local/share/velune-observability`，权限 0700 |
| 配置 | [compose.yaml](deployment/compose.yaml)，远端使用同一内容 |
| 镜像 | `grafana/otel-lgtm:0.38.1@sha256:0df9bc14142000b161344bd24caf6ddd319b7319055aedaf9b0692b125730af7` |
| 容器／数据卷 | `velune-observability-lgtm-1`／`velune-observability_data`，卷挂载到 `/data` |
| Grafana | sfp7 回环 `127.0.0.1:30300`，本开发机通过 SSH 隧道访问 [Grafana](http://127.0.0.1:30300) |
| OTLP/gRPC | sfp7 回环 `127.0.0.1:34317`，本开发机隧道同端口，无额外 OTLP metadata；加密与访问授权由 SSH 承担 |
| 资源预算 | 4 GiB 内存、2 CPU；这是保护上限，不是最低需求 |
| 重启／关停 | `unless-stopped`；组件关停宽限 40 秒，Docker 宽限 45 秒 |
| 保留 | Prometheus 3 天／2 GiB block 保留阈值；Loki、Tempo 72 小时；Pyroscope 72 小时但没有发送 profiles |

只发布 Grafana 与 OTLP/gRPC 的回环端口。OTLP/HTTP、各后端端口均未发布；不能把 `docker ps` 中显示的镜像 EXPOSE 端口误当成宿主监听。没有开启 eBPF、主机 PID／特权模式、云转发或额外插件预安装。已关闭各组件可配置的使用统计上报；未做完整外连抓包审计。

Grafana 匿名访问已关闭，`admin` 使用新生成的随机密码，保存在远端 `private.env`（0600）。专用 service account `velune-agent` 是 Viewer，其 90 天查询 token 保存在远端 `query-token.json`（0600）。本开发机私有副本在仓库 `target/otlp-backend-probe/query-token.json`；`target` 被 Git 忽略，实验目录为 0700。认证值不写入文档、命令参数或报告。到期后在 Grafana 的 service account 设置中轮换 token，并替换私有副本。

开发通道已归 `svc dev` 管理；入口、目标与本地覆盖见[开发环境](../../docs/development.md#开发环境)。本机执行 `svc dev ensure telemetry`，Windows 通道执行 `svc dev ensure telemetry-windows`，Android 执行 `svc dev ensure telemetry-android`。停止使用同名 `svc dev stop`，不删除数据卷。SSH control socket 位于 `~/.local/state/velune/dev/`；旧 `target/otlp-backend-probe/ssh.sock` 已停止，不再作为启动入口。

管理服务时先 SSH 到 sfp7，在部署目录运行：

```sh
docker compose --env-file private.env ps
docker compose --env-file private.env logs --tail 100 lgtm
docker compose --env-file private.env stop
docker compose --env-file private.env up -d
```

`stop` 保留数据与容器；`up -d` 恢复运行。不要用 `down -v` 做普通停止，它会删除数据卷。容器日志轮转为每文件 10 MiB、最多 3 份。保留时间和 Prometheus block 阈值都不是整个数据卷的磁盘硬配额；WAL、活跃块与删除延迟仍会占空间，后续开发应观察卷大小。

## agent 查询入口

通过 Grafana 的 datasource proxy 使用 `Authorization: Bearer <私有查询 token>`。客户端上报无需、也不应携带这个查询 token。已配置的 datasource UID 为 `tempo`、`loki`、`prometheus`，统一入口前缀是 `http://127.0.0.1:30300/api/datasources/proxy/uid/`。

| 目标 | 路径与查询 |
| --- | --- |
| Trace ID | `tempo/api/traces/<trace-id>`，请求 JSON |
| TraceQL | `tempo/api/search`，参数 `q`、`start`、`end`、`limit` |
| 日志 | `loki/loki/api/v1/query_range`，参数 `query`、`start`、`end`、`limit` |
| 指标 | `prometheus/api/v1/query`，参数 `query`、`time`；区间查询使用 `query_range` |

合成 trace 可用 `b3bbdf62b965446c96571c6324817d89`；其 run ID 是 `probe-9dfa71d253de42d2887a9658536bc524`。日志查询为 `{service_name="velune-probe-executor"} |= "probe-9dfa71d253de42d2887a9658536bc524"`，PromQL 为 `velune_probe_operations_total{probe_run_id="probe-9dfa71d253de42d2887a9658536bc524"}`。使用[证据文件](deployment/evidence.json)中的发送时间查询历史值；超过保留期后应重新发送，不把正常过期当作故障。

可复验的[合成探针](deployment/probe.py)只依赖官方 OTLP protobuf 与 gRPC 库，不是产品 exporter。实验 venv 位于 `target/otlp-backend-probe/venv`；实际安装版本为 `grpcio==1.85.0`、`opentelemetry-proto==1.45.1`、`protobuf==7.36.2`、`typing_extensions==4.16.0`。在仓库根目录、隧道就绪后运行：

```sh
target/otlp-backend-probe/venv/bin/python tasks/otlp-observability/deployment/probe.py send target/otlp-backend-probe/manual.json
target/otlp-backend-probe/venv/bin/python tasks/otlp-observability/deployment/probe.py verify target/otlp-backend-probe/manual.json
```

写入接受与查询可见之间存在异步批处理延迟；`verify` 若尚不可见，稍后重试。每次 `send` 生成新 run ID。此处能验证后台通路，不能证明 Swift／Rust／Kotlin／.NET SDK 已兼容。

## 实测结果

2026-10-26 使用纯合成数据，经当前开发机的真实 SSH 隧道测试，记录见[结构化证据](deployment/evidence.json)。

- 查到两个 service 的父子 spans、同 trace ID 的日志，以及 counter=7、histogram count=3、sum=0.35；TraceQL 搜索也命中预期 trace。
- 在 40.006 秒内按每秒 5 次操作发送 400 spans、200 logs、400 metric data points，同时做 8 轮三信号查询。每次 OTLP 返回均无拒绝；最终 counter=1400、histogram count=600、sum=70。
- 55 秒资源采样取得 109 个样本，Docker 报告内存 418.1–485.5 MiB；CPU 峰值为单核的 16.58%，均值 7.57%。负载前后 `/data` 已分配大小约从 67.0 MiB 增至 69.0 MiB；这个短测包含后台自身指标，不能外推长期增长或生产容量。
- 正常关停并强制重建容器，容器 ID 发生变化，保留同一数据卷。重建后旧三信号仍能查询，新三信号也通过；未出现 OOM 或自动重启。主机重启没有实测。
- 匿名、错误 token 和默认 `admin:admin` 均返回 401；Viewer 查询返回 200，修改 datasource 返回 403，经 proxy 尝试空 Prometheus／Loki 写入也返回 403。非回环地址上的两个发布端口均拒绝连接。此身份用于可信开发 agent，不把开发 LGTM 宣称为生产多租户隔离。
- 已读取运行中的 Prometheus、Loki、Tempo 配置，确认上述保留值生效；未等待 72 小时观察实际删除。Pyroscope 参数经对应版本 help 和正常启动核对，未发送或查询 profiles。

四端与真机结果见上述验收记录；长期故障恢复、长期资源上限与磁盘清理尚未完整验证。页面登录和图形交互没有作为此次验收依据；已验证的是 agent 使用的机器 API。
