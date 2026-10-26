# 桌面端全键盘操作

2026-10-26 用户要求桌面端不借助鼠标也能高效使用，同意落地 A～F 全部，并指定运行中 Return 排队、⌘Return 插入本轮。决定见 [S49](../../docs/sources.md#s49)，产品归 [键盘操作](../../docs/prd/keyboard.md)。

## 范围

- A 全局导航：命令面板、打开会话、转到待处理的会话、全局唤起、从通知进入。
- B 审批：输入区上方的卡片取代 sheet，数字键选择，不抢焦点。
- C 输入区：发送方式设置、插入本轮、历史调出、⌥↑ 排队消息、`/` 命令、`@` 文件、附件与下一轮选择的快捷键、Esc 分层。
- D 消息列表：转到消息、消息间移动、展开收起、拷贝、编辑。
- E 会话列表：Return 重命名、删除、输入即搜索。
- F 底层：作用域、冲突检测、速查表、Windows 对应。

先在 Mac 落地，Windows 按同一 PRD 跟进并更新 [对齐表](../../app/windows/PARITY.md)。

## 方案

- 核心不变：队列、审批、发送、steer 与编辑都已有生成接口；本任务只改平台界面。`/` 只列 Velune 命令，Harness 原生命令目录需要核心提供运行时声明，不在本任务。
- 命令定义一处：Mac 的 `ShortcutCommand`、Windows 的 `Command` 表同时供菜单、命令面板、`/` 命令与速查表使用。
- 发送方式是应用级设置，与命令快捷键存于同一文件（Mac `mac-shortcuts.json`、Windows `windows-shortcuts.json`），schema 不变，新增字段缺省为 Return 发送。
- Esc 由一处键盘路由按 PRD 的层级处理，输入法组词时直接交给文本框。

## 验证

不新增自动化测试。Mac 构建零警告；开发版用临时 `VELUNE_HOME`、合成会话与预览数据，在后台进程内检查按键路由与呈现，不读取真实会话。Windows 在 win-ws 上 `scripts\build-windows.ps1` 通过，只截取 Velune 自己的窗口。实际手感由用户验收。

## 进度

- 2026-10-26：记录决定、PRD 与本任务包。
