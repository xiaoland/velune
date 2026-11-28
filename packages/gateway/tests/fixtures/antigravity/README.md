# Antigravity ACP 回环测试数据

`wire.json` 取自或改编自 Magpie Community `agy-cli` 的 MIT 测试样本，固定在
`e77dec9f3946e796e54375252aa28f2c1a18c419`。[上游许可证](./LICENSE)

- ACP 初始化结构、旧版模型列表与选择、提示完成来自
  [testkit.mjs](https://github.com/magpie-community/plugins/blob/e77dec9f3946e796e54375252aa28f2c1a18c419/packages/agy-cli/test/testkit.mjs#L266-L344)。
  `authentication_required` 保留其中记录的 agy 1.5.3 未登录错误对象。
  旧版模型样例来自 [testkit.mjs 的模型列表](https://github.com/magpie-community/plugins/blob/e77dec9f3946e796e54375252aa28f2c1a18c419/packages/agy-cli/test/testkit.mjs#L77-L89)。
- Unicode 块、回显工具参数及新版模型选项来自
  [smoke.mjs 的流事件](https://github.com/magpie-community/plugins/blob/e77dec9f3946e796e54375252aa28f2c1a18c419/packages/agy-cli/test/smoke.mjs#L350-L356)、
  [工具样本](https://github.com/magpie-community/plugins/blob/e77dec9f3946e796e54375252aa28f2c1a18c419/packages/agy-cli/test/smoke.mjs#L424-L435)和
  [模型样本](https://github.com/magpie-community/plugins/blob/e77dec9f3946e796e54375252aa28f2c1a18c419/packages/agy-cli/test/smoke.mjs#L460-L525)。

`fake_agy.py` 用 Python 3 标准库把样本作为 ACP 子进程回复。工具只请求测试网关提供的
本机 HTTP MCP 地址。模型清单和工具结果属于合成数据。子进程在指定步骤退出，以模拟连接中断。
[smoke.mjs 的进程退出用例](https://github.com/magpie-community/plugins/blob/e77dec9f3946e796e54375252aa28f2c1a18c419/packages/agy-cli/test/smoke.mjs#L296-L299)。
