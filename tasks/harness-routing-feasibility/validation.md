# 2026-10-02 无凭据验证记录

基线：`28d13ec522d0bb1dbd44edb10d34662644cc881a`。本记录随实现提交；最终交付 commit 由 Git HEAD／源码 bundle 标识，不在自身提交内容中循环嵌入 hash。

环境：Linux x86_64 Codex Cloud；官方 Rust 1.99.0 (b940084d7 2026-09-28)、Cargo 锁定依赖、bundled SQLite。未运行真实 Harness、未读取秘密、未调用模型、未 push／PR／部署。

| 检查 | 实际结果 |
| --- | --- |
| cargo fmt --check | 通过 |
| cargo check --locked | 通过 |
| cargo clippy --locked --all-targets -- -D warnings | 通过 |
| cargo test --locked | 8 项集成测试通过：vertical 7 + host 1，无失败 |
| cargo build --locked --release | Linux release 成功 |
| bash -n scripts/build-macos.sh | shell 语法通过；没有执行 macOS 构建 |
| release CLI demo | 3 会话、4 attempts、2 验收 done 子任务、0 native calls |
| 本地 Markdown 相对链接／锚点及 git diff 检查 | 通过，提交前再次核对 |

实际 CLI 输出已回读核对：3 个会话、4 个完成 attempts、2 个 done 子任务。原始运行输出／数据库不进入源码包，可用复现命令重建。`simulation:true` 不可移除；协议标签只表示 mock Router 的选择，不能解释为真实网络请求。UI 不拥有第二套任务模拟逻辑。

测试内容见 [vertical.rs](../../tests/vertical.rs) 与 [host.rs](../../tests/host.rs)：

- Codex→Claude Code／Pi 委派，真实执行确定性输入检查、结果回到 Codex 模拟会话后验收；重复投递不重复消费，复用 key 改 payload 拒绝。
- busy／明确未发送保留队列；deadline 过期停止；跨 scope 和 adapter 绑定错误拒绝。
- 提交后异常模拟丢回执，关闭／重开 SQLite 后记 unknown；同 Segment 不重放或再执行其他消息；第二 Host 不能取得独占连接。
- accepted 队列与已生成结果跨重启恢复；账户粘性、资源资格与真实资源禁止规则；错误结果不当作 done。
- 启动实际 Rust Host 子进程，用 Unix socket 请求；取消待执行、断开客户端仍继续、显式停止、重启 epoch 增长及历史保持。

未验收：Mac AppKit 编译、布局／交互、ad-hoc 签名启动、用户设备体验；真实 Harness 协议保真、请求覆盖、原生恢复、MCP／ACP、安全隔离、真实预算。必须由父会话转移本地提交到 Mac，依 [设备步骤](../../docs/development.md#mac-mini-原生体验交付契约)执行并记录反馈。不能将源码和 Linux 测试等同设备完成。
