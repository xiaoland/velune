# Velune 的 mDNS 接口范围补丁

本目录是官方 [iroh-address-lookups](https://github.com/n0-computer/iroh-address-lookups/tree/6f103e7a92ddbd2007383527ab5024c983e3c1c6/iroh-mdns-address-lookup) 的 `iroh-mdns-address-lookup` 0.6.0 窄副本，基线固定为 `6f103e7a92ddbd2007383527ab5024c983e3c1c6`。沿用 MIT OR Apache-2.0 双许可，许可证原文随目录保存。保留该提交的缓存更新修复；不复制整个上游 workspace、dev-dependencies、自动化测试、示例、bench 或 CI。`src/lib.rs` 的整个 `cfg(test)` 模块已删除，doctest 入口关闭。

`builder.ipv4_interfaces(BTreeSet<Ipv4Addr>)` 开启显式接口模式。空集合创建稳定的 lookup actor，但不创建任何 discoverer、IPv4／IPv6 multicast socket，也不退回默认接口；非空集合使用 Swarm 的 `with_multicast_interfaces_v4` 和 `V4Only`。未调用此 builder 方法时，保留上游自动选择行为。Velune 调用方始终使用显式模式。

`set_ipv4_interfaces(set).await` 的成功结果是 actor 已应用的代际，初始代际为 0。成功应用的相同集合不重建；改变集合会 drop 整个旧 `DropGuard`，推进代际并重建 discoverer。旧 callback 即使已排进 actor 队列也会被代际检查拒绝。外层 actor、registry 中的 service、Endpoint 与订阅保持稳定。重建失败时返回错误，并停留在没有 discoverer 的状态，不恢复旧范围；同一个失败集合可以重试。

每次接口代际变化都清除本 service 的旧 peer 缓存并发送 `DiscoveryEvent::Expired`。这个事件也表示旧接口范围的记录失效，不能解释为远端设备已经离线。保留旧记录会导致暂停时失去过期事件来源，却永远返回旧地址。已有 resolve 窗口仍保持原截止时间，可以接收新代际的发现结果；订阅不需要重新创建。最近 publish 的 EndpointData 保留并在恢复发现时重新应用。

`DropGuard::drop` 只调用 guardian task 的 `abort`；Acto 的 `ActoCell::drop` 随后 abort supervised children。代际确认不等于取消已完成、socket 已关闭或网络广告已撤回。本地有界人工实验可以观察自有进程的 5353 FD 最终归零，但这个观察不是同步 API 保证。

Swarm 0.6.4 的 `Sockets::new` 即使传入非空接口集合，也会建立 wildcard IPv4 接收 socket；接口发送 socket 创建失败只记 warning，全部失败时可能使用主 socket 发送。因此，此补丁不声称非空模式是严格网络接口隔离。显式空集合直接跳过 `Discoverer::spawn`，不会触发这些默认行为。

## 重现与撤回

从固定提交取出 `iroh-mdns-address-lookup/src/lib.rs`，在第一个 `\n#[cfg(test)]\nmod tests {` 处分割，保留前半段、去除末尾空白并追加一个换行；随后在独立目录用 `patch -p1 < runtime.patch` 应用本目录的 [runtime.patch](runtime.patch)。该 patch 只记录生产 runtime 的变化，不包含被移除的测试正文。

Cargo 元数据来自同提交的 package `Cargo.toml`：去掉指向上游 CI 的注释与 `[lints] workspace = true`，删除整个 `[dev-dependencies]` 区块，在 dependencies 前加入 `[lib] doctest = false`；其余 package、runtime dependencies、docs.rs 元数据保持原样。两份 LICENSE 从上游仓库根目录逐字复制。主仓库只通过 package path 使用此副本，不为其建立独立 workspace 或 lock。

当官方稳定版本同时提供显式空集合暂停、IPv4 接口重建、旧 callback 代际隔离与明确缓存失效语义，并在 Velune 的 Android／Mac 人工验收中满足同样判据后，移除此目录及 path 装配，恢复官方依赖。上游仅提供动态地址移除或发送停止，不足以证明 discoverer task 与 socket 生命周期满足这些要求。
