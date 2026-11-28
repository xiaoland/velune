# 工作区冷开与恢复

## 操作范围

首次打开已有会话、短暂失败后重试、恢复本设备工作区以及新建和关闭标签。

## 用户入口

启动 Velune 后从会话列表打开既有会话。在工作区新建和关闭会话标签，重启后查看恢复的窗口、分组及标签。

## 用原生检查复验

```sh
python3 scripts/check-macos-composer.py experience --workspace-only --app "$app" --output "$out"
```

核对 `PASS cold open`、`PASS workspace journal` 与 `PASS window shell`。检查临时错误后仍指向原会话、恢复范围限定设备、AppKit 响应链实际完成开关标签。

## 易漏条件

这里走后台外壳和命令路径。判断系统真实键盘焦点、菜单或可见窗口生命周期时，按 Mac Agent 指引为该行为选择对应证据。
