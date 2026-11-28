# Antigravity CLI 额度样本

`usage.json` 参照 Magpie 公开测试中的 `LIVE_REPORT`。原样本来自 Antigravity CLI 1.5.9 的 `agy -p /usage --output-format json`。此处保留成功封套、bucket ID、周期及其中一个非整数精度样值，使用合成重置时间和其余比例。测试不需要登录、网络或真实 `agy` 可执行文件。[Magpie 固定版本的样本](https://github.com/magpie-community/plugins/blob/90bd010d57dc576ccadf1f4eabd1ac67fce5743d/packages/agy-cli/test/usage.mjs#L12-L47)

`models.json` 使用同一公开测试的 `LIVE_MODEL_ROWS` 中部分型号和名称，装入 ACP `session/new` 的旧版 `models.availableModels` 形状，session ID 为合成值。测试也以这组数据构造新版分组选择项。额度属于哪个模型取决于同一采集目标当次返回的目录，旧版名称、共享组和独立 Argon 组都只匹配有证据的 ID。[Magpie 模型目录样本](https://github.com/magpie-community/plugins/blob/90bd010d57dc576ccadf1f4eabd1ac67fce5743d/packages/agy-cli/test/usage.mjs#L48-L68) [Google CLI 模型目录格式](https://antigravity.google/docs/cli/headless/#select-a-model-effort-or-agent)
