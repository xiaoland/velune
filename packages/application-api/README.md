# 应用业务契约

`velune-application-api` 保存本地原生界面与远程客户端共用的业务输入、结果、错误和投影。它只依赖中立的 `conversation`、Serde、base64 与错误显示库，不连接 Harness、网关或提供商，也不读取配置、环境或机器文件。执行实现归 [application](../application/README.md)，网络身份与请求接纳归 [remote](../remote/README.md)。

`Command` 穷尽现有应用公开业务：会话、发送队列、消息编辑、审批、变更、分析、提供商／模型／运行时、认证编辑及资源管理；每项有同名 `Response` 结果。`ReleaseView` 只释放当前视图的临时状态。`ApplicationReply` 同时给出本次执行后的 `Revision`；远程修改配置、删除或替换内容时，`ExecutionContext.expected_revision` 必须引用最近观察的版本。版本不匹配返回 `FailureKind::Conflict` 与当前版本，不覆盖另一设备的改动。原生资源写入继续使用它自己的内容 revision。

审批的 `interaction.id` 是待决请求的 opaque 身份，不是 Harness 的原始请求编号。执行 adapter 为每个实际请求发行会话随机 epoch 加单调序号，并保留只在请求仍待决时可用的原始编号映射；答复或失效即消费。不同设备和重复投影看到同一身份，原生编号在后来轮次复用也不会恢复旧审批。

`Failure` 的分类、`code` 和 `phase` 保留 I/O、契约、历史、提供商来源、原生资源、陈旧审批和执行生命周期差别；传输失败与执行结果未知不伪装为业务失败。命令和回复不实现 `Debug`，含密钥、环境、资源内容和附件的结构继续遮蔽内容；普通提供商摘要不带认证材料。运行时摘要省略 ACP 启动参数与环境，显式 `AcpLaunch` 编辑读取仍可返回这些值。API 模型不包含内部 Pi projection，模板快填仍是独立快照。

`ClientViewId` 是宿主分配的视图标识。远程入口须先用认证设备身份限定它；不能让两个设备自报的相同视图名共享选择或草稿。已配对设备拥有完整业务权限，查看 API key、认证交互、完整资源和附件仍通过具名显式操作；这些内容不自动进入发现或配置摘要。

附件通过 `UploadAttachment { name, bytes }` 提交内容并取得 `AttachmentReference.reference`。远程 `ComposedTurn.attachments` 只接受当前视图取得的引用；本地调用仍接受执行机器绝对路径。`ReadAttachment` 只按会话、消息与块序号读取真实投影中的图片或文件，不接受任意路径。图片投影经远程传递时转换为内容；大图片先返回 `Deferred`，再通过 `MessagePayloads` 取得完整内容。二进制字段在 JSON 中采用 base64，原生语言 API 保持 `Vec<u8>`。上传及内容读取上限为 20 MiB，资源条目另遵守 adapter 自己的边界。

本 unit 不拥有请求 ID、可信设备、连接、持久会话或跨崩溃去重记录；远程库管理请求接纳与结果查询，执行端应用拥有业务状态。断线后缺失回执需要查询实际接纳状态，不能由序列化契约推断可以安全重发。
