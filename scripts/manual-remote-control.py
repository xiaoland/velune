#!/usr/bin/env python3
"""显式隔离验收远控；只使用临时 HOME、合成 ACP peers 与本机 iroh。

这不是自动化测试或 CI 入口。请显式提供同一次 v2 构建的动态库、Python bindings
和 resources；不读取用户的 Velune 配置、Harness 认证或真实会话。夹具在核对双方
派生码后显式调用执行端本地批准 API，不代表产品自动授信。
"""
import argparse
from contextlib import contextmanager
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile
import threading
import time


def wait_for(observe, description, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = observe()
        if value:
            return value
        time.sleep(0.05)
    raise AssertionError('超时：' + description)


def rejected(operation, error_type):
    try:
        operation()
    except error_type as error:
        return error
    raise AssertionError('应拒绝的操作成功了')

@contextmanager
def remote_operation(client):
    """显式处理完本次 typed 结果后确认；异常退出保留准确 pending ID。"""
    operation = client.prepare_remote_operation()
    yield operation.application()
    operation.acknowledge()


def texts(b, snapshot):
    return [block.text for message in snapshot.messages for block in message.blocks
            if isinstance(block, b.BindingMessageBlock.TEXT)]


def write_peers(root):
    """复用 ACP 真实协议 fixture，加入由隔离 gate 释放的异步 turn。"""
    fixtures = Path(__file__).resolve().parent
    collaboration = runpy.run_path(str(fixtures / 'manual-collaboration.py'), run_name='fixture')
    peer = runpy.run_path(str(fixtures / 'manual-acp-adapter.py'), run_name='fixture')['PEER']
    peer = peer.replace('import json, os, sys, uuid', 'import json, os, sys, uuid, threading, time, hashlib, base64')
    permission = "permission='permission-'+uuid.uuid4().hex"
    assert peer.count(permission) == 1
    peer = peer.replace(permission, "permission='reused-native-permission'")
    capability = "        result(identity,{'protocolVersion':1,'agentInfo':{'name':'fixture'"
    assert peer.count(capability) == 1
    peer = peer.replace(capability, "        capabilities['promptCapabilities']={'image':True}\n" + capability)
    capture = "text=text,model=model)"
    assert peer.count(capture) == 1
    peer = peer.replace(capture, "text=text,model=model,images=[{'media_type':item.get('mimeType'),'bytes':len(base64.b64decode(item['data'])),'sha256':hashlib.sha256(base64.b64decode(item['data'])).hexdigest()} for item in params['prompt'] if item['type']=='image'])")
    marker = "session['records'].append(['user',text]);save()"
    assert peer.count(marker) == 1
    peer = peer.replace(marker, marker + ";text=text.split('\\n\\n[Velune 协作消息]',1)[0]")
    marker = "        if text=='EOF':"
    assert peer.count(marker) == 1
    peer = peer.replace(marker, '''        if text == 'GATE':
            def release(identity=identity):
                deadline = time.monotonic() + 30
                while (root / 'hold').exists() and time.monotonic() < deadline:
                    time.sleep(.02)
                answer(identity, 'DISCONNECTED_TURN_COMPLETED')
            threading.Thread(target=release, daemon=True).start()
        elif text=='EOF':''')
    (root / 'alpha.py').write_text(collaboration['ALPHA'])
    (root / 'beta.py').write_text(peer)


def configure_peers(b, app, root):
    write_peers(root)
    execution = b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),
        home=b.BindingRuntimeHome.MANAGED(remote_root=None), remote_resources_directory=None)
    for identity, name, launch in (
        ('alpha', 'Alpha', b.BindingAcpLaunch(args=[str(root / 'alpha.py')],
            environment={'ALPHA_STATE': str(root / 'alpha-state'), 'PRIVATE_MARKER':'SYNTHETIC_ACP_ENV_SECRET'})),
        ('beta', 'Beta', b.BindingAcpLaunch(args=[str(root / 'beta.py'), 'literal ; $(not-a-shell)'],
            environment={'ACP_MODE': 'full', 'ACP_STATE': str(root / 'beta-state')})),
    ):
        app.upsert_acp_runtime(b.BindingRuntimeInstance(enabled=True, id=identity, name=name,
            type_id='acp-v1', gateway_id='default', settings={'binary': sys.executable}), execution, launch)


def git_fixture(root):
    directory = root / 'project'
    directory.mkdir()
    env = {'HOME': str(root / 'user-home'), 'PATH': '/usr/bin:/bin',
        'GIT_CONFIG_NOSYSTEM': '1', 'GIT_AUTHOR_NAME': 'Synthetic',
        'GIT_AUTHOR_EMAIL': 'synthetic@example.invalid', 'GIT_COMMITTER_NAME': 'Synthetic',
        'GIT_COMMITTER_EMAIL': 'synthetic@example.invalid'}
    def git(*arguments):
        subprocess.run(['git', '-C', str(directory), *arguments], env=env, check=True,
            capture_output=True, text=True)
    git('init', '-q', '-b', 'main')
    (directory / 'file.txt').write_text('before\n')
    git('add', 'file.txt')
    git('commit', '-q', '-m', 'synthetic base')
    (directory / 'file.txt').write_text('after\n')
    return directory


def exercise_business(b, local, first, second, root, report):
    """走现有具名业务接口，远端结果由本地唯一 owner 交叉观察。"""
    configure_peers(b, local, root)
    first.expect_revision(first.list_with_revision().revision)
    second.expect_revision(second.list_with_revision().revision)
    project = git_fixture(root)
    model = b.BindingProviderModel(record_key='', provider_model_id='synthetic', nickname='Synthetic',
        icon=None, context_window=8192, max_output_tokens=128, reasoning_levels=None)
    provider = b.BindingProviderDraft(id='synthetic', name='Synthetic',
        protocol=b.BindingGatewayProtocol.MESSAGES_V1, endpoint='http://127.0.0.1:9/v1', models=[model],
        quota_limits=[], execution_backend=b.BindingExecutionBackend.HTTP(),
        request_profile=b.BindingRequestProfile.NATIVE,
        authentication_source=b.BindingProviderAuthenticationSource.API_KEY,
        quota_source=b.BindingProviderQuotaSource.NONE)
    with remote_operation(first) as scoped:
        scoped.save_provider('default', provider, b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_REMOTE_SECRET'))
    assert first.read_provider_api_key('default', 'synthetic') == 'SYNTHETIC_REMOTE_SECRET'
    assert local.read_provider_api_key('default', 'synthetic') == 'SYNTHETIC_REMOTE_SECRET'
    # The typed configuration deliberately exposes metadata and presence, not key contents.
    summary = first.list()
    assert 'SYNTHETIC_REMOTE_SECRET' not in repr(summary)
    assert any(provider.id == 'synthetic' for gateway in summary.gateways for provider in gateway.providers)
    report.append('configuration_shared_owner_and_explicit_secret')

    assert 'SYNTHETIC_ACP_ENV_SECRET' not in repr(summary)
    alpha_runtime = next(runtime for runtime in summary.runtime_instances if runtime.id == 'alpha')
    assert 'acpLaunch' not in alpha_runtime.settings
    launch = first.acp_launch('alpha')
    assert launch.environment['PRIVATE_MARKER'] == 'SYNTHETIC_ACP_ENV_SECRET'
    alpha_runtime.name = 'Alpha edited'
    with remote_operation(first) as scoped:
        scoped.upsert_runtime(alpha_runtime)
    assert first.acp_launch('alpha').environment == launch.environment
    assert first.acp_launch('alpha').args == launch.args
    report.append('acp_environment_explicit_only_and_runtime_edit_preserves_launch')
    # The same named method works without a scope; a definite outcome is acknowledged by Rust.
    alpha_runtime.name = 'Alpha unscoped'
    first.upsert_runtime(alpha_runtime)
    assert next(r for r in local.list().runtime_instances if r.id == 'alpha').name == 'Alpha unscoped'
    report.append('unscoped_effect_records_then_acknowledges')

    with remote_operation(first) as scoped:
        scoped.upsert_runtime(b.BindingRuntimeInstance(enabled=False, id='resources', name='Resources',
            type_id='claude-code-2.1', gateway_id='default', settings={'agentDir':str(root / 'resource-agent')}))
    kind, scope = b.BindingRuntimeResourceKind, b.BindingRuntimeResourceScope
    catalog = first.runtime_resource_catalog('resources', None, kind.USER_INSTRUCTIONS, scope.USER)
    resource = catalog.entries[0].key
    revision = b.BindingRuntimeResourceRevision
    with remote_operation(first) as scoped:
        created = scoped.write_runtime_resource('resources', None, resource, revision.MISSING(), b'FIRST\n', None)
    other = second.read_runtime_resource('resources', None, resource)
    assert other.bytes == b'FIRST\n' and other.entry.revision == created.entry.revision
    with remote_operation(first) as scoped:
        changed = scoped.write_runtime_resource('resources', None, resource, revision.MATCH(created.entry.revision), b'SECOND\n', None)
    with remote_operation(second) as scoped:
        rejected(lambda: scoped.write_runtime_resource('resources', None, resource,
            revision.MATCH(other.entry.revision), b'STALE\n', None), b.BindingError.Diagnostic)
    assert first.read_runtime_resource('resources', None, resource).bytes == b'SECOND\n'
    assert local.read_runtime_resource('resources', None, resource).entry.revision == changed.entry.revision
    report.append('remote_resource_read_write_and_stale_revision')

    manifest = b'---\nname: fixture\ndescription: Synthetic fixture\n---\nFixture.\n'
    for path, data in (('fixture/SKILL.md', manifest), ('fixture/assets/data.bin', b'\x00\xff\x80')):
        key = b.BindingRuntimeResourceKey(kind=kind.SKILLS, scope=scope.USER, path=path)
        with remote_operation(first) as scoped:
            scoped.write_runtime_resource('resources', None, key, revision.MISSING(), data, None)
        assert second.read_runtime_resource('resources', None, key).bytes == data
    imported = first.import_runtime_resource('resources', kind.SKILLS, 'fixture')
    assert next(file.bytes for file in imported.template.content.files if file.path == 'assets/data.bin') == b'\x00\xff\x80'
    first.expect_revision(first.observed_revision())
    with remote_operation(first) as scoped:
        saved_template = scoped.save_resource_template(imported.template)
    recovered = second.read_resource_template(saved_template.summary.id)
    assert next(file.bytes for file in recovered.content.files if file.path == 'assets/data.bin') == b'\x00\xff\x80'
    stored = json.loads((root / 'application' / 'resource-templates.json').read_text())
    stored_skill = next(template for template in stored['templates'] if template['id'] == saved_template.summary.id)
    assert next(file['bytes'] for file in stored_skill['content']['files'] if file['path'] == 'assets/data.bin') == [0,255,128]
    report.append('binary_skill_asset_native_and_template_storage_round_trip')

    default = None  # No choice: the runtime uses its own default.
    with remote_operation(first) as scoped:
        created = scoped.create_conversation_with_selection('beta', str(project), default).snapshot
    identity = created.conversation.id
    assert second.snapshot('beta', identity).snapshot.conversation.id == identity
    changes = first.working_changes('beta', identity)
    assert changes.repository == str(project) and any(file.path == 'file.txt' for file in changes.files)
    diff = second.file_diff('beta', identity, 'file.txt')
    assert (b.BindingDiffLineKind.ADDED, 'after') in [(line.kind, line.text) for line in diff.lines]
    rejected(lambda: first.file_diff('beta', identity, '../outside'), b.BindingError.Diagnostic)
    report.append('remote_conversation_and_working_changes')

    def snapshot(client=first):
        return client.snapshot('beta', identity).snapshot
    with remote_operation(first) as scoped:
        scoped.send_turn_with_selection('beta', identity, 'beta', default,
            b.BindingComposedTurn(text='PERMIT', effort=None, attachments=[]))
    approval = wait_for(lambda: snapshot().pending_interactions, '远端审批出现')[0]
    assert second.snapshot('beta', identity).snapshot.pending_interactions[0].id == approval.id
    replies = []
    start = threading.Barrier(2)
    def answer(client):
        start.wait()
        with remote_operation(client) as scoped:
            try:
                scoped.reply_runtime_interaction('beta', identity, approval.id,
                    b.BindingRuntimeInteractionReply.DECISION(option_id='allow'))
                replies.append('accepted')
            except b.BindingError.Diagnostic:
                replies.append('rejected')
    workers = [threading.Thread(target=answer, args=(client,)) for client in (first, second)]
    for worker in workers:
        worker.start()
    for worker in workers:
        worker.join(timeout=15)
        assert not worker.is_alive(), '审批请求未结束'
    assert sorted(replies) == ['accepted', 'rejected'], replies
    settled = wait_for(lambda: (value if value.actions.can_send else None)
        if (value := snapshot()) else None, '审批后正常结束')
    assert 'ACP_PERMISSION' in texts(b, settled)
    report.append('two_remote_clients_compete_for_one_actual_approval')

    with remote_operation(first) as scoped:
        scoped.send_turn_with_selection('beta', identity, 'beta', default,
            b.BindingComposedTurn(text='PERMIT', effort=None, attachments=[]))
    next_approval = wait_for(lambda: snapshot().pending_interactions, '原生审批 ID 复用的新 turn')[0]
    assert next_approval.id != approval.id
    assert second.snapshot('beta', identity).snapshot.pending_interactions[0].id == next_approval.id
    with remote_operation(second) as scoped:
        rejected(lambda: scoped.reply_runtime_interaction('beta', identity, approval.id,
            b.BindingRuntimeInteractionReply.DECISION(option_id='allow')), b.BindingError.Diagnostic)
    assert snapshot().pending_interactions[0].id == next_approval.id
    with remote_operation(first) as scoped:
        scoped.reply_runtime_interaction('beta', identity, next_approval.id,
            b.BindingRuntimeInteractionReply.DECISION(option_id='allow'))
    wait_for(lambda: snapshot().actions.can_send, '新 turn 的有效审批完成')
    report.append('reused_native_approval_id_gets_distinct_public_id_and_old_reply_cannot_answer_new_turn')
    return identity, default


def exercise_disconnected_work(b, first, second, identity, default, root, disconnect_all, reconnect, report):
    """断线窗口只观察合成 peer 文件；不请求本地或远程 snapshot 驱动核心。"""
    gate = root / 'beta-state' / 'hold'
    gate.write_text('hold')
    with remote_operation(first) as scoped:
        scoped.send_turn_with_selection('beta', identity, 'beta', default,
            b.BindingComposedTurn(text='GATE', effort=None, attachments=[]))
    with remote_operation(first) as scoped:
        scoped.send_queue('beta', identity, b.BindingSendQueueOperation.ADD(
            source_runtime_id='beta', target_runtime_id='beta', selection=default,
            turn=b.BindingComposedTurn(text='QUEUED_WHILE_OFFLINE', effort=None, attachments=[])))
    log = root / 'beta-state' / 'events.jsonl'
    def events():
        return [json.loads(line) for line in log.read_text().splitlines()]
    wait_for(lambda: any(row.get('text') == 'GATE' for row in events()), 'gate turn 进入 peer')
    disconnect_all()
    gate.unlink()
    wait_for(lambda: any(row.get('text') == 'QUEUED_WHILE_OFFLINE' for row in events()),
        '没有客户端时队列真正发出下一 turn')
    second = reconnect('second')
    settled = wait_for(lambda: (value if value.actions.can_send and not value.follow_up.queue else None)
        if (value := second.snapshot('beta', identity).snapshot) else None, '另一端恢复已完成投影')
    assert 'DISCONNECTED_TURN_COMPLETED' in texts(b, settled)
    assert any(row.get('text') == 'QUEUED_WHILE_OFFLINE' for row in events())
    report.append('all_control_clients_disconnected_turn_and_queue_continue')

    with remote_operation(second) as scoped:
        scoped.send_turn_with_selection('beta', identity, 'beta', default,
            b.BindingComposedTurn(text='PERMIT', effort=None, attachments=[]))
    pending = wait_for(lambda: second.snapshot('beta', identity).snapshot.pending_interactions,
        '断线前实际待决审批')[0]
    disconnect_all()
    second = reconnect('second')
    restored = second.snapshot('beta', identity).snapshot
    assert restored.pending_interactions[0].id == pending.id
    with remote_operation(second) as scoped:
        scoped.reply_runtime_interaction('beta', identity, pending.id,
            b.BindingRuntimeInteractionReply.DECISION(option_id='allow'))
    wait_for(lambda: second.snapshot('beta', identity).snapshot.actions.can_send, '恢复审批完成')
    report.append('paired_device_resumes_actual_pending_approval')

    with remote_operation(second) as scoped:
        alpha = scoped.create_conversation_with_selection('alpha', str(root / 'project'), default).snapshot.conversation.id
    with remote_operation(second) as scoped:
        scoped.send_turn_with_selection('alpha', alpha, 'alpha', default,
            b.BindingComposedTurn(text='START Beta offline-collaboration', effort=None, attachments=[]))
    disconnect_all()
    def collaboration_finished():
        for path in (root / 'alpha-state').glob('*.json'):
            try:
                value = json.loads(path.read_text())
            except json.JSONDecodeError:
                continue  # The synthetic peer replaces this tiny observation file in place.
            if any(isinstance(update.get('content'), dict)
                    and 'PEER_REPLIED: ACP_ANSWER' in update['content'].get('text', '')
                    for update in value['updates']):
                return True
        return False
    wait_for(collaboration_finished, '没有控制端时协作子会话完成并回到 Alpha', timeout=45)
    second = reconnect('second')
    result = wait_for(lambda: (value if value.actions.can_send else None)
        if (value := second.snapshot('alpha', alpha).snapshot) else None, '恢复协作最终投影')
    assert any('PEER_REPLIED: ACP_ANSWER' in text for text in texts(b, result))
    report.append('all_control_clients_disconnected_collaboration_continues')
    return second


def exercise_views_and_attachments(b, local, first, second, identity, default, root, report):
    with remote_operation(first) as scoped:
        scoped.show_archived_conversations(False)
    with remote_operation(second) as scoped:
        scoped.show_archived_conversations(True)
    assert not first.list().shows_archived and second.list().shows_archived
    with remote_operation(first) as scoped:
        scoped.select_runtime('alpha')
    with remote_operation(second) as scoped:
        scoped.select_runtime('beta')
    assert first.list().selected_runtime_instance_id == 'alpha'
    assert second.list().selected_runtime_instance_id == 'beta'
    with remote_operation(second) as scoped:
        scoped.show_archived_conversations(False)
    report.append('independent_remote_archive_and_runtime_views')

    controller = root / 'controller-files'
    controller.mkdir()
    notes = controller / 'notes.txt'
    payload = b'SYNTHETIC_REMOTE_ATTACHMENT_CONTENT\n'
    notes.write_bytes(payload)
    before = second.snapshot('beta', identity).snapshot
    with remote_operation(second) as scoped:
        rejected(lambda: scoped.send_turn_with_selection('beta', identity, 'beta', default,
            b.BindingComposedTurn(text='UNSAFE_PATH', effort=None, attachments=[str(notes)])), b.BindingError.Diagnostic)
    assert len(second.snapshot('beta', identity).snapshot.messages) == len(before.messages)
    with remote_operation(second) as scoped:
        uploaded = scoped.upload_attachment(notes.name, notes.read_bytes())
    assert uploaded.name == notes.name and uploaded.bytes == len(payload)
    assert uploaded.reference != str(notes)
    notes.unlink()
    with remote_operation(second) as scoped:
        scoped.send_turn_with_selection('beta', identity, 'beta', default,
            b.BindingComposedTurn(text='UPLOADED_ATTACHMENT', effort=None, attachments=[uploaded.reference]))
    settled = wait_for(lambda: (value if value.actions.can_send else None)
        if (value := second.snapshot('beta', identity).snapshot) else None, '受控上传附件完成')
    user = next(message for message in reversed(settled.messages)
        if message.role == b.BindingMessageRole.USER and any(isinstance(block, b.BindingMessageBlock.TEXT)
            and 'UPLOADED_ATTACHMENT' in block.text for block in message.blocks))
    index = next(index for index, block in enumerate(user.blocks) if isinstance(block, b.BindingMessageBlock.FILE))
    assert user.blocks[index].path != str(notes) and Path(user.blocks[index].path).name == 'notes.txt'
    assert second.read_attachment('beta', identity, user.id, index).bytes == payload
    report.append('controller_path_rejected_and_uploaded_content_survives_controller_deletion')


def exercise_configuration_conflict(b, first, second, report):
    editor = first.list_with_revision()
    other_editor = second.list_with_revision()
    previous = editor.revision
    assert other_editor.revision == previous
    runtime = next(runtime for runtime in editor.configuration.runtime_instances if runtime.id == 'resources')
    runtime.name = 'Resources updated'
    first.expect_revision(previous)
    with remote_operation(first) as scoped:
        scoped.upsert_runtime(runtime)
    runtime.name = 'Stale name'
    second.expect_revision(previous)
    with remote_operation(second) as scoped:
        rejected(lambda: scoped.upsert_runtime(runtime), b.BindingError.Diagnostic)
    assert next(runtime.name for runtime in first.list().runtime_instances if runtime.id == 'resources') == 'Resources updated'
    report.append('stale_configuration_revision_refused')



def exercise_large_image(b, client, identity, default, root, report):
    # 17 MiB, valid PNG signature: this isolates transport/attachment limits, not an image decoder.
    payload = b'\x89PNG\r\n\x1a\n' + bytes(17 * 1024 * 1024 - 8)
    with remote_operation(client) as scoped:
        uploaded = scoped.upload_attachment('large.png', payload)
    with remote_operation(client) as scoped:
        scoped.send_turn_with_selection('beta', identity, 'beta', default,
            b.BindingComposedTurn(text='LARGE_IMAGE', effort=None, attachments=[uploaded.reference]))
    settled = wait_for(lambda: (value if value.actions.can_send else None)
        if (value := client.snapshot('beta', identity).snapshot) else None, '17 MiB 图片发送完成', timeout=45)
    user = next(message for message in reversed(settled.messages)
        if message.role == b.BindingMessageRole.USER)
    image_index = next(index for index, block in enumerate(user.blocks)
        if isinstance(block, b.BindingMessageBlock.IMAGE))
    content = client.read_attachment('beta', identity, user.id, image_index)
    assert content.media_type in (None, 'image/png') and content.bytes == payload
    events = [json.loads(line) for line in (root / 'beta-state' / 'events.jsonl').read_text().splitlines()]
    delivered = next(row for row in reversed(events) if row.get('text') == 'LARGE_IMAGE')
    assert delivered['images'] == [{'media_type':'image/png', 'bytes':len(payload), 'sha256':hashlib.sha256(payload).hexdigest()}]
    before = sorted(str(path) for path in (root / 'application').rglob('*') if path.is_file())
    with remote_operation(client) as scoped:
        rejected(lambda: scoped.upload_attachment('oversized.png', b'\x89PNG\r\n\x1a\n' + bytes(20 * 1024 * 1024 - 7)), b.BindingError.Diagnostic)
    assert sorted(str(path) for path in (root / 'application').rglob('*') if path.is_file()) == before
    assert client.list().gateways, '预算拒绝后连接仍应可用'
    report.append('17_mib_image_upload_send_and_read_content_identical_over_budget_refused')


def exercise_request_reconciliation(b, client, root, disconnect_all, reconnect, acknowledge, report):
    payload = b'SYNTHETIC_ACCEPTED_RECEIPT_DISCARDED\n'
    original_revision = client.observed_revision()
    client.expect_revision(original_revision)
    request = client.next_request_id()
    command = b.BindingApplicationCommand.UPLOAD_ATTACHMENT(name='receipt.txt', bytes=payload)
    # This is received-then-discarded, not a lost wire receipt or an Endpoint replacement proof.
    client.submit_request(request, command)
    disconnect_all()
    def saved():
        return [path for path in (root / 'application').rglob('*') if path.is_file()
            and path.stat().st_size == len(payload) and path.read_bytes() == payload]
    wait_for(saved, '已接纳请求在客户端离线后产生真实暂存副作用')
    assert len(saved()) == 1
    client = reconnect('first')
    status = wait_for(lambda: (value if isinstance(value, b.BindingRemoteRequestStatus.COMPLETED) else None)
        if (value := client.query_request(request)) else None, '断线后按原 ID 找到业务结果')
    reference = status.response.value.reference
    client.expect_revision(original_revision)
    duplicate = client.submit_request(request, command)
    assert isinstance(duplicate, b.BindingRemoteRequestStatus.COMPLETED)
    assert duplicate.response.value.reference == reference and len(saved()) == 1
    rejected(lambda: client.submit_request(request,
        b.BindingApplicationCommand.UPLOAD_ATTACHMENT(name='different.txt', bytes=b'DIFFERENT')), b.BindingError.Diagnostic)
    assert len(saved()) == 1
    unknown = client.next_request_id()
    assert isinstance(client.query_request(unknown), b.BindingRemoteRequestStatus.UNKNOWN)
    acknowledge(request)
    report.append('discarded_receipt_reconciled_same_request_id_does_not_repeat_side_effect_unknown_explicit')
    return client


def exercise_same_object_concurrency(b, client, root, report):
    gate, ready = root / 'slow-hold', root / 'slow-ready'
    gate.write_text('hold')
    source = (root / 'beta.py').read_text()
    marker = "    if method=='initialize':"
    assert source.count(marker) == 1
    source = source.replace(marker, marker + "\n        Path(os.environ['SLOW_READY']).write_text('ready')\n        while Path(os.environ['SLOW_GATE']).exists(): time.sleep(.02)")
    slow_peer = root / 'slow.py'
    slow_peer.write_text(source)
    client.expect_revision(client.list_with_revision().revision)
    with remote_operation(client) as scoped:
        scoped.upsert_acp_runtime(b.BindingRuntimeInstance(enabled=True, id='slow', name='Slow',
            type_id='acp-v1', gateway_id='default', settings={'binary':sys.executable}),
            b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),
                home=b.BindingRuntimeHome.MANAGED(remote_root=None), remote_resources_directory=None),
            b.BindingAcpLaunch(args=[str(slow_peer), 'literal ; $(not-a-shell)'],
                environment={'ACP_MODE':'full', 'ACP_STATE':str(root / 'slow-state'),
                    'SLOW_GATE':str(gate), 'SLOW_READY':str(ready)}))
    allocated = client.next_request_id()
    unknown = b.BindingRemoteRequestId(incarnation=allocated.incarnation,
        client=allocated.client, sequence=allocated.sequence + 1000)
    slow_results, query_results, failures = [], [], []
    def slow():
        try:
            slow_results.append(client.acp_runtime_info('slow'))
        except Exception as error:
            failures.append(error)
    def query():
        try:
            query_results.append(client.query_request(unknown))
        except Exception as error:
            failures.append(error)
    slow_worker = threading.Thread(target=slow)
    query_worker = threading.Thread(target=query)
    slow_worker.start()
    try:
        wait_for(ready.exists, '同一远控对象的慢请求已在实际ACP初始化中等待')
        assert slow_worker.is_alive()
        query_worker.start()
        query_worker.join(timeout=3)
        concurrent = not query_worker.is_alive() and gate.exists() and slow_worker.is_alive()
    finally:
        gate.unlink(missing_ok=True)
        slow_worker.join(timeout=15)
        if query_worker.ident is not None:
            query_worker.join(timeout=15)
    assert not slow_worker.is_alive() and not query_worker.is_alive()
    assert not failures, failures
    assert concurrent, '同一对象的安全查询被慢业务调用持锁阻塞'
    assert len(query_results) == 1 and isinstance(query_results[0], b.BindingRemoteRequestStatus.UNKNOWN)
    assert slow_results[0].agent_info.name == 'fixture'
    report.append('same_remote_object_safe_query_completes_while_real_slow_business_call_waits')


def exercise_stale_message_edit(b, first, second, identity, default, report):
    snapshot = first.snapshot('beta', identity).snapshot
    original = next(message for message in snapshot.messages if message.role == b.BindingMessageRole.USER)
    revision = first.observed_revision()
    with remote_operation(second) as scoped:
        scoped.send_turn_with_selection('beta', identity, 'beta', default,
            b.BindingComposedTurn(text='MESSAGE_STATE_CHANGED', effort=None, attachments=[]))
    wait_for(lambda: second.snapshot('beta', identity).snapshot.actions.can_send, '另一端改变会话后新 turn 完成')
    first.expect_revision(revision)
    with remote_operation(first) as scoped:
        failure = rejected(lambda: scoped.edit_message('beta', identity, original.id,
            b.BindingEditPlacement.IN_PLACE, default,
            b.BindingComposedTurn(text='STALE_MESSAGE_EDIT', effort=None, attachments=[])), b.BindingError.Diagnostic)
    assert failure.kind == b.BindingFailureKind.CONFLICT
    current = second.snapshot('beta', identity).snapshot
    assert not any('STALE_MESSAGE_EDIT' in text for text in texts(b, current))
    assert 'MESSAGE_STATE_CHANGED' in texts(b, current)
    report.append('stale_message_edit_refused_as_conflict_before_harness_dispatch')

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'resources'):
        parser.add_argument('--' + name, required=True, type=Path)
    args = parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists():
            parser.error('需要绝对路径且路径必须存在')
    original_environment = dict(os.environ)
    application = server = None
    clients, endpoints = {}, {}
    report = []
    with tempfile.TemporaryDirectory(prefix='velune-remote-control-') as temporary:
        root = Path(temporary).resolve()
        for name in ('bindings', 'user-home'):
            (root / name).mkdir()
        shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings' / 'velune_bindings.py')
        (root / 'bindings' / args.library.name).symlink_to(args.library)
        os.environ.clear()
        os.environ.update(HOME=str(root / 'user-home'), PATH='/usr/bin:/bin', NO_PROXY='127.0.0.1,localhost')
        try:
            spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings' / 'velune_bindings.py')
            b = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = b
            spec.loader.exec_module(b)
            application = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'application'),
                resources_directory=str(args.resources)))

            def endpoint(name, owner=None):
                directory = root / 'devices' / name
                directory.mkdir(parents=True, exist_ok=True)
                return b.VeluneRemoteEndpoint.open(b.BindingRemoteOptions(
                    identity_file=str(directory / 'identity.json'), state_file=str(directory / 'state.json'),
                    device_name=name, relay=b.BindingRelayConfig.DISABLED(),
                    discovery=b.BindingDiscoveryConfig.NONE(), bind_addresses=['127.0.0.1:0'],
                    request_timeout_ms=30000, path_policy=b.BindingRemotePathPolicy.AUTOMATIC), owner)
            server = endpoint('execution', application)
            descriptor = server.descriptor()
            assert not descriptor.relay_urls and descriptor.direct_addresses
            assert all(address.startswith('127.0.0.1:') for address in descriptor.direct_addresses)
            # A durable outgoing pin alone does not grant control of the execution device.
            rogue = endpoint('unpaired')
            try:
                candidate = rogue.begin_pairing(server.create_invitation(120).encoded)
                assert candidate.phase == b.BindingPairingPhase.AWAITING_APPROVAL
                assert len(rogue.read_state().contacts) == 1
                rejected(lambda: rogue.connect(descriptor.endpoint_id, 'unpaired-view'), b.BindingError.Diagnostic)
                server.reject_pairing(candidate.attempt_id, rogue.descriptor().endpoint_id)
                report.append('pinned_but_unapproved_device_refused')
            finally:
                rogue.shutdown()
            for name in ('first', 'second'):
                endpoints[name] = endpoint(name)
                invitation = server.create_invitation(120)
                candidate = endpoints[name].begin_pairing(invitation.encoded)
                assert candidate.phase == b.BindingPairingPhase.AWAITING_APPROVAL
                incoming = next(value for value in server.read_state().incoming_pairings
                    if value.attempt_id == candidate.attempt_id)
                assert incoming.peer_id == endpoints[name].descriptor().endpoint_id
                assert incoming.verification_code == candidate.verification_code
                assert len(candidate.verification_code) == 8 and candidate.verification_code.isdigit()
                if name == 'first':
                    assert endpoints[name].begin_pairing(invitation.encoded).attempt_id == candidate.attempt_id
                approved = server.approve_pairing(candidate.attempt_id, incoming.peer_id)
                assert approved.phase == b.BindingPairingPhase.GRANTED
                assert endpoints[name].pairing_status(descriptor.endpoint_id).phase == b.BindingPairingPhase.GRANTED
                clients[name] = endpoints[name].connect(descriptor.endpoint_id, name + '-view')
                clients[name].list()
            assert len(server.read_state().incoming_grants) == 2
            assert not server.read_state().contacts
            report.append('v2_local_code_approval_and_two_directional_grants')
            first, second = clients['first'], clients['second']
            path_status = first.connection_status()
            assert not path_status.closed
            assert any(path.selected and path.kind == b.BindingConnectionPathKind.DIRECT for path in path_status.paths)
            path_observations = [{'kind':str(path.kind), 'selected':path.selected,
                'rtt_microseconds':path.rtt_microseconds, 'transmitted_bytes':path.transmitted_bytes,
                'received_bytes':path.received_bytes, 'lost_packets':path.lost_packets}
                for path in path_status.paths]
            report.append('actual_selected_iroh_path_is_direct')
            identity, default = exercise_business(b, application, first, second, root, report)
            assert not endpoints['first'].read_state().pending, 'definite outcomes left pending records'
            exercise_stale_message_edit(b, first, second, identity, default, report)
            exercise_configuration_conflict(b, first, second, report)
            exercise_views_and_attachments(b, application, first, second, identity, default, root, report)
            exercise_large_image(b, second, identity, default, root, report)
            exercise_same_object_concurrency(b, first, root, report)

            def disconnect_all():
                for client in clients.values():
                    client.shutdown()
                clients.clear()
                for control_endpoint in endpoints.values():
                    control_endpoint.shutdown()
                endpoints.clear()
            def reconnect(name):
                endpoints[name] = endpoint(name)
                clients[name] = endpoints[name].connect(descriptor.endpoint_id, name + '-view')
                return clients[name]
            def acknowledge(request):
                endpoints['first'].acknowledge_request(descriptor.endpoint_id, request)
            first = exercise_request_reconciliation(b, first, root, disconnect_all, reconnect, acknowledge, report)
            second = reconnect('second')
            second = exercise_disconnected_work(b, first, second, identity, default, root,
                disconnect_all, reconnect, report)
            server.revoke_control(endpoints['second'].descriptor().endpoint_id)
            # Revocation closes this native connection; its in-flight read can fail at transport.
            rejected(second.list, b.BindingError)
            rejected(lambda: endpoints['second'].connect(descriptor.endpoint_id, 'revoked-view').list(), b.BindingError.Diagnostic)
            report.append('revoke_blocks_existing_and_new_business_connections')
            for path in (root / 'application').rglob('*.jsonl'):
                content = path.read_text()
                assert 'SYNTHETIC_REMOTE_SECRET' not in content and 'SYNTHETIC_ACP_ENV_SECRET' not in content
            report.append('synthetic_secrets_absent_from_jsonl_logs')
            print(json.dumps({'acceptance':'PASSED', 'transport':'iroh loopback QUIC',
                'observations':report, 'paths':path_observations, 'direct_addresses':descriptor.direct_addresses, 'relay_urls':descriptor.relay_urls,
                'public_network_verified':False,
                'real_models_or_credentials_used':False}, ensure_ascii=False, indent=2))
        finally:
            for client in clients.values():
                try:
                    client.shutdown()
                except Exception:
                    pass
            for control_endpoint in endpoints.values():
                control_endpoint.shutdown()
            if server is not None:
                server.shutdown()
            if application is not None:
                application.shutdown_stopping_work()
            os.environ.clear()
            os.environ.update(original_environment)


if __name__ == '__main__':
    main()
