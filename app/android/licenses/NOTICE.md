# Velune Android：声明与第三方组件

本文件随 APK 分发，也可在应用的“关于 › 声明与第三方组件”中查看。

Velune 本身按 GNU Affero General Public License version 3 only（AGPL-3.0-only）发布；完整许可证见“Velune AGPL-3.0-only 许可证”。源代码与对应版本源码归档见 <https://github.com/xiaoland/velune>。

## 界面参考

Android 界面的布局与交互参考了以下同类开源移动客户端。Velune 只借鉴了可见的布局与交互模式，例如按项目分组的会话列表、折叠为一行的工作过程、圆角输入框与其中的选项行、回到底部按钮、带状态点的设备列表。**Velune 没有复制、翻译或改写这些项目的源代码、图片或文案**，相关代码均为独立编写。

| 项目 | 许可 | 参考内容 |
| --- | --- | --- |
| [HAPI](https://github.com/tiann/hapi)（tiann/hapi） | AGPL-3.0 | Android 客户端的会话列表、输入区、审批卡片布局 |
| [Lody](https://github.com/LodyAI/Lody)（LodyAI/Lody） | Apache-2.0 | 文档中移动端截图：工作过程折叠、输入区选项行、回到底部按钮 |
| [Happy](https://github.com/slopus/happy)（slopus/happy） | MIT | 移动客户端的整体信息层级 |

上述名称与标志归各自所有者。界面中的运行时标识（如“CC”“Cx”“Pi”）是字母缩写，不是 Anthropic、OpenAI 等公司的商标标志；Claude Code、Codex 等名称归其各自所有者，Velune 与它们没有从属或认可关系。

## 随应用分发的组件

### Java／Kotlin 库

| 组件 | 许可 |
| --- | --- |
| OpenTelemetry Java SDK、OTLP exporters 与 Kotlin context extension（OpenTelemetry Authors） | Apache-2.0 |
| OkHttp、Okio（Square） | Apache-2.0 |
| Kotlin 标准库、kotlinx.coroutines（JetBrains） | Apache-2.0 |
| AndroidX：Activity、Compose（UI、Foundation、Material 3、Material Icons）、Lifecycle、CameraX 等（Google） | Apache-2.0 |
| ZXing core（ZXing authors） | Apache-2.0 |
| Guava ListenableFuture、AutoValue 注解（Google）、JetBrains annotations、JSpecify | Apache-2.0 |
| JNA（Java Native Access） | Apache-2.0 或 LGPL-2.1+ 双许可，Velune 按 Apache-2.0 使用 |
| rustls-platform-verifier Android 组件 | MIT 或 Apache-2.0 |
| commonmark-java 及其 GFM 表格、删除线、自动链接扩展 | BSD-2-Clause，见下 |
| autolink-java | MIT，见下 |

Apache-2.0 全文见“Apache License 2.0”。

### Rust 组件

原生库 `libvelune_bindings.so` 链接的 Rust crate 及其许可全文见“Rust 组件许可”，由 `scripts/android-licenses.py` 在构建时按实际依赖生成。对应版本源码归档包含 `Cargo.lock` 锁定的 Rust 依赖源码，其中包括未修改的 UniFFI MPL-2.0 源码；MPL-2.0 全文见“Mozilla Public License 2.0”。

### commonmark-java（BSD-2-Clause）

```
Copyright (c) 2015, Robin Stocker
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this
  list of conditions and the following disclaimer.

* Redistributions in binary form must reproduce the above copyright notice,
  this list of conditions and the following disclaimer in the documentation
  and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### autolink-java（MIT）

```
The MIT License (MIT)

Copyright (c) 2015 Robin Stocker

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
