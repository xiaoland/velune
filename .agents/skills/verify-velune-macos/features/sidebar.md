# 工作区侧栏

## 操作范围

检查侧栏和会话列表的控制按钮归属，收起后重开并恢复保存的宽度。

## 用户入口

在工作区工具栏使用侧栏按钮，或使用菜单的显示／隐藏侧栏命令。拖动分割线缩放后复验恢复。

## 用原生检查复验

```sh
python3 scripts/check-macos-composer.py sidebar-toolbar --app "$app" --output "$out"
```

核对 `PASS sidebar toolbar`、`sidebar-toolbar-check.json` 及当次导出的布局图片。通过行应涵盖 AppKit 装配、工具归属、收起重开和保存宽度。

## 易漏条件

结构与命令检查在后台运行。视觉评审需实际查看导出的截图，系统快捷键需对应的事件与焦点证据。
