# 会话读取依赖声明

Velune 使用 huihua 的公开 API 读取 Agent 原生会话文件。依赖通过 npm 安装，未复制上游解析器源码。发行包应同时包含本文件、licenses 目录和 runtime-support-package-lock.json；后者记录具体版本与完整性校验值。

| 包 | 版本 | 许可证 | 完整声明 |
| --- | --- | --- | --- |
| huihua | 0.2.0 | MIT | licenses/huihua-MIT.txt |
| fzstd | 0.1.1 | MIT | licenses/fzstd-MIT.txt |
| xxhashjs | 0.2.2 | MIT | licenses/xxhashjs-MIT.txt |
| cuint | 0.2.2 | MIT | licenses/cuint-MIT.txt |
| @bufbuild/protobuf | 2.16.0 | Apache-2.0 AND BSD-3-Clause | licenses/protobuf-Apache-2.0.txt 与 licenses/protobuf-Google-BSD-3-Clause.txt |

cuint 的 npm 包未提供独立 LICENSE 文件；其 package.json 声明 MIT，源码头部注明 Pierre Curto 的版权。本目录保留该版权与 MIT 完整条款。protobuf 的 npm 包声明组合许可；Apache 条款来自 protobuf-es v2.16.0 仓库 LICENSE，Google BSD 条款来自该发行包 wire/varint.js 的原始许可头部。

Pi、DeepSeek Harness 与 Codex 均由用户另行安装，本依赖包不分发任何 Agent runtime 或完整 Pi SDK。

## models.dev public catalog

User-requested model template suggestions come from https://models.dev/api.json.
The models.dev catalog is MIT licensed, copyright (c) 2025 models.dev. Its license
is included in `licenses/models.dev-LICENSE`. Source provider and model IDs are
retained; suggestions are copied to independent, user-editable templates.
