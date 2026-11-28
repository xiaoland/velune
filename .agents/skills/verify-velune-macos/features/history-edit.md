# 编辑历史消息

## 操作范围

编辑正文与附件，保持当前草稿独立，拒绝已过期的编辑，提交到开始编辑时的会话。

## 用户入口

打开已有会话，在已发送用户消息的操作行点编辑。修改内容后从历史编辑器提交。

## 用原生检查复验

```sh
python3 scripts/check-macos-composer.py editor --app "$app" --output "$out"
```

核对 `PASS offscreen native editor`、`editor-native.json`、`editor-wire.jsonl`。日志必须包含过期内容被拒和正常提交定位到原会话的结果。涉及外观时查看本次截图。

## 易漏条件

普通历史编辑使用离屏合成数据。改动 Claude 压缩边界时运行 `claude-compact-edit`，需要已安装的 Claude Code，检查原会话保留、替换范围和 `loopback.json` 的请求。后一项走真实 CLI 与回环上游。
