# iroh 1.3.0 运行源码补丁

此目录固定供应 iroh 1.3.0 单个 crate，来源为 crates.io 发布包。精确 archive URL、SHA-256、上游提交和许可证文本来源见 [SOURCE.json](SOURCE.json)。运行源文件、build.rs、规范化 Cargo.toml 和原包 LICENSE-BSD3 来自该发布包；MIT、Apache-2.0 双许可证文本取自同一上游提交的仓库根目录。没有供应 iroh-relay、noq 或上游 workspace。

[tools/runtime-source.py](tools/runtime-source.py) 验证原包 SHA-256 后重建运行源码基线：删除 tests、examples、benches、test-utils 模块与 feature、仅其使用的可选依赖、dev-dependencies 和 cfg(test) 项，关闭 doctest，再使用 SOURCE.json 记录的 rustfmt 格式化。运行逻辑变更单独保存在 [runtime.patch](runtime.patch)，不混入上述供应裁剪。

在仓库根目录以本地可信原包和新的临时目标目录重放：

```sh
python3 third-party/iroh/tools/runtime-source.py /path/to/iroh-1.3.0.crate /tmp/iroh-runtime-baseline
patch -d /tmp/iroh-runtime-baseline -p1 < third-party/iroh/runtime.patch
diff -r /tmp/iroh-runtime-baseline/src third-party/iroh/src
diff /tmp/iroh-runtime-baseline/Cargo.toml third-party/iroh/Cargo.toml
```

## 功能边界

`endpoint::Builder::relay_auth_tokens` 接收 `(RelayUrl, String)` 迭代器，替换本 Endpoint 私有且不可变的认证表。重复完整 URL 的最后一项生效。表不增加 RelayMap 条目、不改变 home relay 选择、路径选择或 relay 协议。现有 relay actor 启动实际连接时按规范化完整 RelayUrl 精确查表，通过原有 ClientBuilder.auth_token 发送认证；不存在域名、路径前缀或默认 token 匹配。

独立表的同 URL 条目优先于旧 RelayConfig.auth_token；无条目时保留上游同 URL 配置的原有语义。Velune 只使用独立表，因此未知 URL 没有可回退的默认 token。表容器 Debug 不显示键和值；同时屏蔽连接选项中的 auth_token 与 actor 中 ClientBuilder 的派生 Debug，避免从内部状态打印认证正文。

本补丁不实现动态 token refresh。Velune 更新认证配置时替换整个网络代际，并等待旧 Endpoint 退出；不能将取消请求或 abort 当成资源已经释放。该退出边界由 packages/remote 管理，不由此补丁另建传输栈。

当上游提供“独立于 home relay 候选的精确 URL 认证”且保持上述 Debug 保密边界时，升级并验证等价行为后删除本目录及根 Cargo patch。任何后续上游升级均需先重建运行基线再重新审视 runtime.patch，不能直接覆盖供应源码。
