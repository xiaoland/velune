# Velune Mac 原生检查场景

在[验证 skill](../SKILL.md)中先创建输出目录并核对开发包，再执行对应页面的命令。其他场景以 `python3 scripts/check-macos-composer.py --help` 和[项目验证说明](../../../../docs/development/validation.md#mac-读取恢复复验)为准。

| 场景 | 可判断的结果 |
| --- | --- |
| [编辑历史消息](history-edit.md) | 编辑状态隔离、过期拒绝与原会话提交 |
| [工作区冷开与恢复](workspace.md) | 暂时读取失败后的单次恢复、正确的窗口和标签 |
| [工作区侧栏](sidebar.md) | 按钮归属、收起与重开、保存宽度 |
| [会话消息列表](transcript.md) | 消息行和 AppKit 辅助功能顺序、单元格复用、缩放重排 |
