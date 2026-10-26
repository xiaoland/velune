# 独立用量显示与 Codex 原生项目

用户于 2026-10-26 明确要求实现：额度与上下文可独立选择“占用／剩余”，支持 Codex 原生多文件夹项目。本轮授权覆盖必要源码、测试和文档修改；用户随后授权提交本轮改动，并明确不为并发风险增加限制或验证；本轮不覆盖安装。

实现从核心配置与运行时适配开始，再接入 Mac、Windows 现有界面。旧的统一用量偏好作为两个选项的初值，保留配置和客户端视图隔离。额度上限仍按已用比例执行，颜色仍反映实际占用。Codex 项目来源先核实安装版本的原生协议，再接入现有项目模型；不修改用户真实 Codex 数据。

验证使用合成配置与会话，覆盖两个偏好独立保存、重启、视图隔离和失败回滚；项目覆盖多个根、原生会话归属与 Velune 显式组织记录的优先级。完成 Rust 验证和平台构建，并记录可完成的界面检查与缺口。

## 实现与决定

用户补充要求在 Velune 内直接编辑 Codex 原生项目。已核实本机 Codex 0.159.3 导出的协议与实际合成调用：`project/list` 返回原生 ID、名称、有序根；`thread.projectId` 拥有归属。顾问复核也实测了共享首根和零根项目，因此没有把路径当作原生身份。

核心保留 `usage_display` 作为额度偏好，新增 `context_usage_display`，旧配置读取时继承旧值。两项贯穿本机、portable、视图基线和远程命令。Mac/Windows 设置与状态栏独立消费；额度判定和颜色不变。

原生项目以运行时实例与原生 ID 组成稳定键，`primary_folder` 单独暴露可选首目录。Mac/Windows 直接调用原生编辑；“显示 → 编辑项目”覆盖没有会话的项目。四端目录建议不再把项目键当路径。原生名称/根编辑直接调用 project/update，仅提交名称与根并保留其他 metadata；按用户要求移除保存前 project/read 和并发冲突拒绝检查。Velune 显式移动只改变组织记录。未加入原生项目创建或原生线程成员重分配入口。

## 当前证据

- Rust `usage_tests` 在完整核心及 no-default-features 下各 2 项通过：继承、独立保存、重启、视图隔离、远端不改本地默认、保存失败回滚。
- `manual-codex-projects.py` 使用临时 HOME 和回环 Responses 服务，五组断言全部通过：共享首根不合并、第二根/根外 cwd 按原生归属、零根项目；原生编辑保留身份与 metadata；归档/取消归档保持归属；不同运行时中的同 ID 项目隔离；Velune 移动与继续保留原生归属、cwd 和旧双根。
- Mac/Windows 最终完整构建通过；iOS Simulator 构建、Android assembleDebug + lintDebug 通过。随后仅合并一处等价 Rust 条件分支，application 与 agent-runtime 的 all-targets Clippy（-D warnings）通过；bindings cargo check 通过。
- Mac 合成配置实际界面确认“额度剩余、上下文占用”同时保留；从“显示 → 编辑项目”修改没有会话的原生项目名称，并从双根项目移除首根和最后一个根，保存、重开结果正确。临时测试 App 正常退出，进程检查确认结束，已安装 App 未被替换。
- 原始模拟响应重复使用 item ID 导致测试应用崩溃，已将夹具改为每次响应独立 ID，并重新通过原生继续测试；未把夹具问题混作产品修复。
- Windows 浅/深色实际界面与正常退出未验证，当前没有连接到 win-ws 的原生界面自动化通道。移动端本轮只做构建验证。

本轮实现与验证已完成，按用户授权提交；未覆盖安装。移除并发检查后，bindings Release 构建、原生项目五组回归与 Clippy 全部重新通过，日志为 /tmp/velune-usage-projects-direct-save-{build,check,clippy}.log。此次未重复四端界面构建与操作验证。文档相对链接与锚点已检查，git diff --check 通过。构建日志位于 /tmp/velune-usage-projects-{mac-final,windows-final,ios-build,android-build}.log，原生回归与静态检查分别为 native-check.log、clippy-final.log（同一前缀）。
