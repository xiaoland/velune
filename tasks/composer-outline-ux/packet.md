# Composer 与大纲体验

## 当前任务

2026-10-10 用户要求改进 composer 的 UI/UX、composer 状态按会话保存、两种大纲呈现（弹窗与原地缩起，均以用户消息为条目）的体验，尤其是原地缩起动画；修正被误解为“消息列表 · 默认显示”的设置，它实际决定点击大纲按钮的行为；“回到底部”按钮补足与滚动条的右边距。

讨论后用户决定：

- 原地模式下点击用户消息：退出折叠、恢复完整列表并停在该消息。
- 弹窗式改为挂在工具栏按钮上的 popover。
- 设置命名“大纲按钮：弹出面板／就地折叠”。
- 草稿需在重启后保留。
- 本轮只做 Mac 小步（边距、大纲语义与动画、按会话 composer 状态）与 composer 界面重做；“核心按会话维护下一轮意图、原生／managed 模型选择、ACP 配置项”需另行对齐，本轮不做。
- 大纲设置持久化字段改名，按 hard-cutoff 将普通配置 schema 升至 8；用户已知旧配置（提供商、API key、运行时实例、模型模板）会被重置并需要重新录入。

稳定需求已写入 [PRD](../../docs/prd/index.md)。授权沿用持续迭代：可改 application／bindings／Mac，自主本地提交并重建安装；不 push、不新增自动化测试、不读取真实会话或凭据、不调用真实提供商。

## 观察（2026-10-10，main `b72e777`）

- 草稿是根视图 `@State`，切换会话后跟随，⌘Return 会发到新会话。下一轮运行时／模型是窗口级状态，打开会话时不使用快照的 `modelRecordKey`。
- 设置 `TranscriptPresentation`（conversation／userOutline）被实现为常驻显示模式；工具栏按钮始终打开 sheet。
- 原地展开／折叠直接重建行，没有过渡；只有锚点行位置保持。
- 回到底部按钮在包含滚动条的区域上只留 16pt，始终显示滚动条时与滚动条几乎相贴。
- composer 有常驻“输入消息”说明行、无占位提示、固定 88pt 高，状态提示插在输入框上方导致布局跳动。

## 方案

- 核心：`OutlinePresentation { popover, inPlace }` 取代 `TranscriptPresentation`，schema 8。草稿是 `VELUNE_HOME/composer-drafts.json`（schema 1，会话公开身份 → 文字），与普通配置分离，不随配置重置；删除会话时一并移除。
- Mac 大纲：根视图持有折叠状态与导航请求；popover 模式在按钮处弹出原生大纲列表，就地模式按钮为 toggle。消息列表的折叠／展开以行高插值动画实现（显示链驱动，约 0.3s），锚点用户消息保持屏幕位置，缩起／出现的行裁剪并淡出／淡入；减少动态效果时直接切换。动画中的滚动、内容更新与宽度变化先完成动画再处理。
- composer：NSTextView 包装，原生占位、随内容增高至上限后滚动、⌘Return 发送且输入法组字时不发送、撤销历史按会话分开；输入与操作行放在同一圆角容器，去掉说明行，状态并入操作行。草稿与运行时／模型按会话保存（运行时／模型只在内存，打开会话时从快照初始化）。

## 验证计划

cargo fmt／check／clippy；Swift 严格构建；扩展 `manual-transcript-scroll.py` 覆盖折叠／展开动画结束后的几何与锚点；临时 HOME 下用绑定检查草稿持久化与删除清理；构建安装后核对 manifest／签名。

## 实施与验证（2026-10-10）

- core：`OutlinePresentation`、schema 8、`composer-drafts.json` 与删除会话时清理；fmt／check／clippy（workspace all-targets all-features）及 `--no-default-features` 检查无警告。
- Mac：Swift warnings-as-errors 构建通过。`manual-transcript-scroll.py` 新增就地大纲场景：无窗口（直接切换）与可见窗口（动画）都通过；折叠后只剩用户消息、锚点位置不变（起始在视口上方时落到顶部）、动画结束不残留裁剪／透明度、点击气泡展开时被点行位置不变、工作过程展开状态跨折叠保留、动画中滚动立即结束动画。可见窗口截图确认折叠中途的淡出与裁剪以及展开后锚点位置。
- 临时脚本（未提交）在 preview store 与临时 HOME 的真实核心上确认：下一轮运行时／模型按会话保存并从快照初始化；schema 7 配置被重置为 8；草稿 0600、重开保留、空文字删除、`draft:` 会话不写入；Store 启动读取草稿并在停顿后写入。
- `manual-transcript-outline.py`（隔离 Pi 1.0.2＋本机合成上游）改用新接口后通过。
- preview 模式整窗截图确认输入区布局、启动聚焦、随内容增高与焦点环。

## 安装

clean `e52848a` release 构建已安装 `/Applications/Velune.app`：manifest source_commit 一致、dirty=false、0.1 beta.1、config schema 8；严格签名通过，主程序与核心库 SHA 与构建一致。正常退出旧应用后安装并重新打开，无崩溃报告。首次启动按 schema 8 hard-cutoff 重置普通配置（用户已同意），原 Harness 会话与认证、`conversation-links.json` 和草稿文件不受影响。

## 用户后续反馈

用户看到新 composer 后认为体验更好，但要求尽可能贴近 macOS 原生风格，同时照顾多行输入、格式编辑等与传统聊天 composer 不同的需求。当前圆角卡片、圆形图标按钮与带图标菜单偏向聊天应用，原生化方案待与用户对齐后实施。
