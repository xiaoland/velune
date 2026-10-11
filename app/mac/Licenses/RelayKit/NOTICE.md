
## RelayKit 协议转换

Velune 使用 [New API](https://github.com/QuantumNous/new-api) 的 RelayKit 模块进行异协议 LLM 请求与响应转换。版本由同目录 go.mod 和 go.sum 固定。模块通过应用内部桥接嵌入；未包含 New API 的管理后台。

Copyright (c) QuantumNous and contributors.

Frontend design and development by New API contributors.

上句是 New API 要求保留的上游署名，不表示 Velune 的原生界面来自 New API。原项目链接：https://github.com/QuantumNous/new-api 。

RelayKit 遵循 GNU Affero General Public License v3.0，完整条款与上游声明见 RelayKit/new-api-LICENSE.txt 和 RelayKit/new-api-NOTICE.txt。Velune 的桥接代码负责内存归属、请求生命周期和传输终态边界，上游转换源码通过 Go 模块使用。

随库链接的 Go runtime 与传递依赖保留以下原始许可。文件均位于 RelayKit 目录，Go 模块具体版本同时记录在 go.mod／go.sum。

| 组件 | 版本 | 许可证文件 |
| --- | --- | --- |
| Go runtime | 构建版本见 build-manifest.json | Go-LICENSE.txt |
| github.com/google/uuid | v1.6.0 | uuid-LICENSE.txt |
| github.com/samber/lo | v1.53.0 | lo-LICENSE.txt |
| github.com/tidwall/gjson | v1.19.0 | gjson-LICENSE.txt |
| github.com/tidwall/sjson | v1.2.5 | sjson-LICENSE.txt |
| github.com/tidwall/match | v1.1.1 | match-LICENSE.txt |
| github.com/tidwall/pretty | v1.2.0 | pretty-LICENSE.txt |
| golang.org/x/text | v0.35.0 | x-text-LICENSE.txt |
