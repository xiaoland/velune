# 开发构建速度与空间

## 目标、授权与当前状态

用户希望改善Rust lib到四端App的开发速度和空间，尤其是多Agent并行；2026-11-06确认“同意，可以一并落地”。实现、受影响构建与定向旧缓存回收均获授权，不包含安装／退出日常应用。工作区其他会话改动全部保留；用户随后明确授权提交，本次提交只覆盖构建优化及其文档、验证脚本和任务记录。

方案见[设计与调查](design.md)，实施后的权威操作归[开发构建与缓存](../../docs/development/builds.md)。当前源码实现、清理后Mac增量构建及文档检查已完成。历史上只读委派漏掉`.build`的盘点和过早空间排序已撤回，采用随后Cargo完整分项。

## 已完成实现

- Cargo默认host缓存为`target/host-rust`，dev／test使用debug=1且保留增量。平台remote使用`target/cross-rust`，轻型UniFFI CLI独立为`tools/bindgen`，缓存`target/build-tools`，不再链接业务库来构建工具。
- 四端SVC dev／release入口；开发默认Debug，安装独立。相同在途请求通过现有SVC合并，平台输出锁保护不同profile共享的生成路径。
- 共用输出工具负责不变不写、绑定配套、来源记录；源码mtime／inode捕获临时改回ABA。输入变化返回3，不覆盖此前成功记录。iOS单一native phase支持首次无生成目录；Android skip-native在Gradle前校验packaged库、许可证与AAR；Windows固定生成器按revision＋SHA认领复用。
- Mac已签名prepared dylib按源码SHA、签名身份与产物SHA复用，App继续最终签名／deep verify。开发脚本不再安装，重装脚本明确release。
- 有限缓存域报告／预算预览／apply，维护先取得平台与Cargo锁。Windows锁使用允许DELETE共享的句柄，持有锁期间可回收闲置锁文件，修复真实WinError32；未为此重复App编译。

## 分层证据

证据在 `target/verification/build-performance`；Windows在 `target/verification/build-performance-20261106/windows`。前期盘点聚合见本包JSON。

| 观察 | 结果 |
| --- | --- |
| 独立生成工具首次build | 约28秒，后续复用；新工具缓存约0.47GiB |
| Mac首次dev | 199秒，含native冷编译1m56与首次Swift编译 |
| Mac无变化 | 最终签名prepared修正后SVC22.864秒，exit0；之前owner／follower两请求同execution约24.9秒 |
| Mac仅UI | SVC23.830秒，不重编Rust／绑定；独立无行为probe已移出 |
| iOS无变化 | 29.36秒，native0.67秒，无crate重编／SwiftCompile；生成文件mtime稳定 |
| Android无变化 | 20.53秒，native0.34秒；Gradle46项up-to-date，输出mtime稳定 |
| Windows首次dev | 439.41秒，native6m10 |
| Windows最终无变化 | 36.885秒，native0.85秒；540项hash／mtime／文件ID稳定 |

原生链的四类变化由真实薄Rust／UniFFI／Swift fixture补齐：无变化保持输入，内部实现变化native SHA改变而Swift／header字节与mtime不变，公开函数增加后生成文件变化，实际Swift消费者调用新接口成功。fixture首轮约18秒，后续各轮不到1秒；没有修改产品ABI或因此重复四端构建，临时fixture缓存已回收。

共用输出／配套／来源检查4组通过，含临时改回ABA、失败保留和旧success不覆盖。缓存检查在Mac和真实Windows均通过；Windows追加idleCargo锁apply，用KB级目录修复并复验，没有恢复完整App链。Mac dev真实bundle的既有controls6组通过（隐藏窗口，无前台激活／全局输入）。Android缺失／损坏packaged库在Gradle前拒绝。iOS首次将生成目录移出后Xcode成功规划构建。

初期实现过程中source变化导致exit3的构建只说明防止误认当前结果生效，不计为当前源码通过。Windows部分末尾helper来源改进通过独立runtime检查，不重验已证明的native／绑定／App生产链。最后新增的Cargo默认target配置以metadata核对，与平台已显式指定的target一致，未因此重复全部App矩阵。

## 已执行空间收敛

五个已验证替代的旧缓存域debug、ios-rust、ios-bindgen、android-bindgen及根级Android triple全部安全回收。约252.5GiB目录分配空间，回收窗口卷可用空间增加269,158,158,336字节（约250.7GiB，受同期系统磁盘活动影响）。debug包含801,452个文件，扫描＋回收418秒，维护不是每次build自动执行。

新热缓存实测：host约4.60GiB、cross约6.15GiB、共用工具约0.47GiB，总计约11.2GiB。旧release、SwiftPM开发索引、下载cache、runtime-support、可运行App、生成目录、用户状态及其他任务证据均保留。普通Cargo使用新默认路径，避免继续生成旧根debug。Windows新热cache约3.85GiB移回日常仓库的标准空位置，临时worktree与junction已安全清理。

回收日志和前后卷观测归`cache-reclamation.json`及`prune-*.log`。新cache没有因回收被清空；最后对Mac做一次回收后增量构建确认依赖完整：31.615秒、exit0/current:true、changed_inputs为空，native库SHA不变。无变化样本因此报告为约23–32秒范围，不挑选最短值冒充稳定保证。

## 实际限制与交付边界

没有可公平对比的旧链冷／热基准，不宣称加速百分比；数字是新链当前机器上的实测。四端release分支作profile／入口静态核对，未再冷编四套release或做分发签名；旧release交付策略保留。未安装、退出或前台运行日常应用，也未执行真实账户／模型请求。直接Xcode与CLI同工作树并行的边界已写入iOS README。旧索引等剩余可再生缓存没有混入本次退役清理。

最后静态收尾移除Mac release分支的空Bash数组（系统Bash3.2与nounset不兼容），两种配置统一使用相同jobs参数；开发调用参数不变。支持层／缓存检查复验通过，shell语法、SVC配置及226个Markdown本地链接／锚点通过，diff无空白错误。
