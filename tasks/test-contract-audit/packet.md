# 测试与验收契约审计

独立工作树为 `.worktrees/0680/velune`，任务分支为 `feat/test-contract-audit`，交付至以 `develop` 为基线的 [PR #17](https://github.com/xiaoland/velune/pull/17)。

首轮从 `2d37a98c7` 开始。按用户授权推送当时的 develop，继续接入上游变更，首轮结果保存在[首轮报告](audit.md)。重新审查以远端 `d2c8e35c0` 为比较基线，以合入该基线的 `b47863cf2` 为只读盘点和实际运行起点。两轮使用同一任务工作树。原已取消任务保留在另一工作树。

[重审报告](reaudit.md) 记录交付范围、实现、维护成本、故障对照和执行边界。[逐声明表](declarations.tsv) 的文件和行号指向 `b47863cf2`，便于在删改后仍能定位；[运行时保护位置](runtime-keepers.md) 解释表中的代号。[新增检查](new-checks.tsv) 另记重审中新增或更名的 Rust、Python 声明、独立判断与实跑结果。机器日志与临时故障注入放在本工作树 `tmp/test-contract-audit/redo/`，生成绑定清单位于 `target/verification/acceptance/`，原生 Mac 回执位于 `target/verification/test-contract-audit/redo/`。
