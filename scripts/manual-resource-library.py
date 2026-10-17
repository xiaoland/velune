#!/usr/bin/env python3
"""手动验收资源模板库与受管同步的 UniFFI 契约；不接入 CI。

只用禁用的合成运行时实例与临时目录，不启动 Harness、不调用模型、不读取真实配置。
"""
import argparse
import gc
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time


PEER = r'''
import json, os, sys, uuid
from pathlib import Path
state = Path(os.environ['FIXTURE_STATE'])
state.mkdir(parents=True, exist_ok=True)
session = None
pending = None

def record(kind):
    with (state / 'events.jsonl').open('a') as output:
        output.write(json.dumps({'kind': kind, 'pid': os.getpid()}) + '\n')

def emit(value):
    print(json.dumps({'jsonrpc': '2.0', **value}), flush=True)

record('start')
for line in sys.stdin:
    request = json.loads(line)
    method, identity = request.get('method'), request.get('id')
    params = request.get('params', {})
    if method == 'initialize':
        emit({'id': identity, 'result': {'protocolVersion': 1,
              'agentInfo': {'name': 'management-lifecycle-fixture', 'version': '1'},
              'agentCapabilities': {'mcpCapabilities': {'http': True}}}})
    elif method == 'session/new':
        session = str(uuid.uuid4())
        record('new')
        emit({'id': identity, 'result': {'sessionId': session}})
    elif method == 'session/prompt':
        assert params['sessionId'] == session
        record('prompt')
        if params['prompt'][0]['text'] == 'CANCEL':
            pending = identity
        else:
            emit({'method': 'session/update', 'params': {'sessionId': session,
                  'update': {'sessionUpdate': 'agent_message_chunk',
                             'content': {'type': 'text', 'text': 'FIXTURE_REPLY'}}}})
            emit({'id': identity, 'result': {'stopReason': 'end_turn'}})
    elif method == 'session/cancel':
        if pending is not None:
            emit({'id': pending, 'result': {'stopReason': 'cancelled'}})
            pending = None
    elif identity is not None:
        emit({'id': identity, 'error': {'code': -32601, 'message': 'unsupported'}})
'''



def exercise(app, b, root):
    kind = b.BindingRuntimeResourceKind
    content = b.BindingResourceTemplateContent
    outcome = b.BindingTemplateApplyOutcome
    state = b.BindingResourceSyncState

    for identity, variant in [('claude', 'claude-code-2.1.286'), ('codex', 'codex-0.159.3'),
                              ('pi', 'pi-1.0.2')]:
        app.upsert_runtime(b.BindingRuntimeInstance(
            enabled=False, id=identity, name=identity, type_id=variant,
            gateway_id='default', settings={'agentDir': str(root / identity)}))

    def resource(name, body, identity=''):
        return b.BindingResourceTemplate(id=identity, name=name, content=body)

    def skill(text, extra=()):
        files = [b.BindingTemplateSkillFile(path='SKILL.md', bytes=text, executable=False)]
        files += [b.BindingTemplateSkillFile(path=p, bytes=t, executable=False) for p, t in extra]
        return content.SKILL(files=files)

    def target(identity, overwrite=False):
        return b.BindingTemplateApplyTarget(runtime_id=identity, overwrite=overwrite)

    def reject(operation):
        try:
            operation()
        except b.BindingError.Diagnostic:
            return
        raise AssertionError('应被拒绝')

    def outcome_of(reports, identity):
        return next(r.outcome for r in reports if r.runtime_id == identity)

    def states(resource_id):
        return {s.runtime_id: s.state for s in app.resource_sync_status() if s.template_id == resource_id}

    def unmanaged(category):
        return {(e.runtime_id, e.path) for e in app.unmanaged_runtime_resources(category)}

    def skill_text(identity):
        return next((root / identity).rglob('review/SKILL.md')).read_bytes()

    # 资源：摘要不含内容；名称与同类唯一；类别不可改。
    review = app.save_resource_template(resource(
        'review', skill(b'---\nname: review\ndescription: d\n---\nv1\n', [('notes/a.md', b'A')]))).summary
    assert review.kind == kind.SKILLS
    reject(lambda: app.save_resource_template(resource('review', skill(b'x'))))
    reject(lambda: app.save_resource_template(resource('bad name', skill(b'x'))))
    reject(lambda: app.save_resource_template(resource('nofile', content.SKILL(files=[]))))
    reject(lambda: app.save_resource_template(resource('review', content.INSTRUCTIONS(body='x'), review.id)))
    print('PASS 资源保存与校验', flush=True)

    # 状态矩阵：未启用；Pi 支持技能。
    assert all(isinstance(v, state.NOT_ENABLED) for v in states(review.id).values())
    reports = app.apply_resource_template(review.id, [target('claude'), target('codex')])
    assert isinstance(outcome_of(reports, 'claude'), outcome.CREATE)
    assert isinstance(states(review.id)['claude'], state.SYNCED)
    assert isinstance(states(review.id)['pi'], state.NOT_ENABLED)
    assert isinstance(outcome_of(app.apply_resource_template(review.id, [target('claude')]), 'claude'), outcome.UNCHANGED)
    print('PASS 启用与状态矩阵', flush=True)

    # 保存即同步：已启用且同步的运行时立即写入，移除的文件被清理。
    saved = app.save_resource_template(resource(
        'review', skill(b'---\nname: review\ndescription: d\n---\nv2\n'), review.id))
    assert {r.runtime_id for r in saved.reports} == {'claude', 'codex'}
    assert b'v2' in skill_text('claude') and b'v2' in skill_text('codex')
    assert not any((root / 'claude').rglob('notes/a.md'))
    assert all(isinstance(states(review.id)[r], state.SYNCED) for r in ('claude', 'codex'))
    print('PASS 保存即同步、移除的文件被清理', flush=True)

    # 原生被改：保存跳过该运行时；三种处置。
    skill_root = next((root / 'claude').rglob('review/SKILL.md')).parent
    (skill_root / 'SKILL.md').write_bytes(b'---\nname: review\ndescription: d\n---\nhand edit\n')
    saved = app.save_resource_template(resource(
        'review', skill(b'---\nname: review\ndescription: d\n---\nv3\n'), review.id))
    assert {r.runtime_id for r in saved.reports} == {'codex'}
    assert b'hand edit' in skill_text('claude') and b'v3' in skill_text('codex')
    assert isinstance(states(review.id)['claude'], state.DRIFTED)
    app.resolve_resource_binding('claude', review.id, b.BindingResolution.OVERWRITE)
    assert b'v3' in skill_text('claude') and isinstance(states(review.id)['claude'], state.SYNCED)
    (skill_root / 'SKILL.md').write_bytes(b'---\nname: review\ndescription: d\n---\nkeep\n')
    app.resolve_resource_binding('claude', review.id, b.BindingResolution.UNMANAGE)
    assert isinstance(states(review.id)['claude'], state.NOT_ENABLED)
    assert ('claude', 'review') in unmanaged(kind.SKILLS)
    reports = app.apply_resource_template(review.id, [target('claude')])
    assert isinstance(outcome_of(reports, 'claude'), outcome.CONFLICT)
    app.apply_resource_template(review.id, [target('claude', True)])
    assert ('claude', 'review') not in unmanaged(kind.SKILLS)
    (skill_root / 'SKILL.md').unlink()
    assert isinstance(states(review.id)['claude'], state.MISSING)
    saved = app.save_resource_template(resource(
        'review', skill(b'---\nname: review\ndescription: d\n---\nv4\n'), review.id))
    assert not (skill_root / 'SKILL.md').exists(), '缺失的运行时等待用户选择，不自动重建'
    app.resolve_resource_binding('claude', review.id, b.BindingResolution.REMOVE)
    assert isinstance(states(review.id)['claude'], state.NOT_ENABLED) and not skill_root.exists()
    print('PASS 原生被改／被删：保存跳过，覆盖／停止同步／移除', flush=True)

    # 指令、MCP 秘密、子 Agent、不支持。
    rules = app.save_resource_template(resource('rules', content.INSTRUCTIONS(body='RULES\n'))).summary
    docs = app.save_resource_template(resource('docs', content.MCP(
        transport=b.BindingTemplateMcpTransport.STDIO(
            command='docs-mcp', args=['--x'], env={'TOKEN': 'SYNTHETIC-SECRET'}, cwd=None),
        timeout_seconds=None))).summary
    helper = app.save_resource_template(resource('helper', content.SUBAGENT(
        description='d', instructions='do it', model=None, velune_model=None, tools=[], effort=None, skills=[]))).summary
    assert 'SYNTHETIC-SECRET' not in repr(app.read_resource_template(docs.id))
    assert oct((root / 'app' / 'resource-templates.json').stat().st_mode & 0o777) == '0o600'
    assert isinstance(states(helper.id)['pi'], state.UNSUPPORTED)
    for item in (rules, docs, helper):
        reports = app.apply_resource_template(item.id, [target('claude'), target('codex')])
        assert not any(isinstance(r.outcome, (outcome.FAILED, outcome.CONFLICT)) for r in reports), reports
    assert (root / 'claude' / 'CLAUDE.md').read_text() == 'RULES\n'
    assert (root / 'codex' / 'AGENTS.md').read_text() == 'RULES\n'
    # 每个运行时只启用一份指令：另一份需确认覆盖，接管后前一份不再启用。
    other = app.save_resource_template(resource('other', content.INSTRUCTIONS(body='OTHER\n'))).summary
    assert isinstance(outcome_of(app.apply_resource_template(other.id, [target('codex')]), 'codex'), outcome.CONFLICT)
    app.apply_resource_template(other.id, [target('codex', True)])
    assert isinstance(states(rules.id)['codex'], state.NOT_ENABLED)
    assert isinstance(states(other.id)['codex'], state.SYNCED)
    print('PASS 指令（每运行时一份）、MCP、子 Agent、不支持', flush=True)

    # 子 Agent 的多行描述、模型、推理强度与技能按各运行时的原生字段写入。
    probe = app.save_resource_template(resource('probe', content.SUBAGENT(
        description='line one\nline two', instructions='probe', model='m-1', velune_model=None, tools=[], effort='minimal', skills=['review'])))
    notes = {r.runtime_id: r.notes for r in app.apply_resource_template(probe.summary.id, [target('claude'), target('codex')])}
    codex_agent = (root / 'codex' / 'agents' / 'probe.toml').read_text()
    claude_agent = (root / 'claude' / 'agents' / 'probe.md').read_text()
    assert 'model = "m-1"' in codex_agent and 'model_reasoning_effort = "minimal"' in codex_agent and 'line two' in codex_agent, codex_agent
    assert any('技能' in note for note in notes['codex']), notes
    # Claude Code 不接受 minimal：跳过并说明；技能写为列表。
    assert 'effort' not in claude_agent and 'model: m-1' in claude_agent and '- review' in claude_agent, claude_agent
    assert any('minimal' in note for note in notes['claude']), notes
    app.save_resource_template(resource('probe', content.SUBAGENT(
        description='d', instructions='probe', model=None, velune_model=None, tools=[], effort='high', skills=['review']), probe.summary.id))
    assert 'effort: high' in (root / 'claude' / 'agents' / 'probe.md').read_text()
    print('PASS 子 Agent 的描述、模型、推理强度、技能按运行时写入，不支持的说明', flush=True)

    # 选 Velune 模型的子 Agent 不写入原生定义，仍是启用状态；改回原生 ID 时写入，再改回时删除。
    app.save_provider('default', b.BindingProviderDraft(
        id='p', name='p', protocol=b.BindingGatewayProtocol.RESPONSES_V1, endpoint='http://127.0.0.1:9/v1',
        models=[b.BindingProviderModel(record_key='', provider_model_id='m-v', nickname='v', icon=None, context_window=None,
                                       max_output_tokens=None, reasoning_levels=None, adapter_metadata_json=None)], quota_limits=[], execution_backend=b.BindingExecutionBackend.HTTP(), request_profile=b.BindingRequestProfile.NATIVE, authentication_source=b.BindingProviderAuthenticationSource.API_KEY, quota_source=b.BindingProviderQuotaSource.NONE),
        b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
    velune_key = app.list().gateways[0].providers[0].models[0].record_key

    def per_turn(model, velune_model):
        return content.SUBAGENT(description='d', instructions='p', model=model, velune_model=velune_model,
                                tools=[], effort=None, skills=[])
    reject(lambda: app.save_resource_template(resource('both', per_turn('m-1', velune_key))))
    reject(lambda: app.save_resource_template(resource('gone', per_turn(None, 'missing-key'))))
    probe = app.save_resource_template(resource('probe', per_turn(None, velune_key), probe.summary.id))
    assert not (root / 'codex' / 'agents' / 'probe.toml').exists() and not (root / 'claude' / 'agents' / 'probe.md').exists()
    assert isinstance(states(probe.summary.id)['codex'], state.SYNCED) and isinstance(states(probe.summary.id)['claude'], state.SYNCED)
    app.save_resource_template(resource('probe', per_turn('m-1', None), probe.summary.id))
    assert 'model = "m-1"' in (root / 'codex' / 'agents' / 'probe.toml').read_text()
    app.save_resource_template(resource('probe', per_turn(None, velune_key), probe.summary.id))
    assert not (root / 'codex' / 'agents' / 'probe.toml').exists()
    # 启用其他资源不影响它。
    app.apply_resource_template(helper.id, [target('codex')])
    assert isinstance(states(probe.summary.id)['codex'], state.SYNCED)
    print('PASS Velune 模型的子 Agent 不写入原生定义而保持启用，与原生 ID 互相切换时写入或删除', flush=True)

    # 运行时中的其他内容与加入资源库。
    (root / 'codex' / 'agents').mkdir(exist_ok=True)
    (root / 'codex' / 'agents' / 'linter.toml').write_text(
        'name = "linter"\ndescription = "lint"\nmodel = "m-2"\nmodel_reasoning_effort = "low"\ndeveloper_instructions = "check"\nextra = 1\n')
    assert ('codex', 'linter.toml') in unmanaged(kind.NATIVE_AGENT_DEFINITIONS)
    preview = app.import_runtime_resource('codex', kind.NATIVE_AGENT_DEFINITIONS, 'linter.toml')
    assert preview.template.name == 'linter' and preview.notes, preview.notes
    assert (preview.template.content.model, preview.template.content.effort) == ('m-2', 'low'), preview.template.content
    assert not any('model' in note for note in preview.notes), preview.notes
    adopted = app.adopt_runtime_resource('codex', kind.NATIVE_AGENT_DEFINITIONS, 'linter.toml')
    assert isinstance(states(adopted.id)['codex'], state.SYNCED)
    assert ('codex', 'linter.toml') not in unmanaged(kind.NATIVE_AGENT_DEFINITIONS)
    adopted_text = (root / 'codex' / 'agents' / 'linter.toml').read_text()
    assert 'extra' not in adopted_text and 'model = "m-2"' in adopted_text and 'model_reasoning_effort = "low"' in adopted_text, adopted_text
    reject(lambda: app.adopt_runtime_resource('claude', kind.MCP_SERVERS, 'docs'))  # 同名已在资源库
    print('PASS 其他内容列表、加入资源库', flush=True)

    # 删除资源保留原生内容；删除运行时清理启用记录。
    app.delete_resource_template(other.id)
    assert (root / 'codex' / 'AGENTS.md').read_text() == 'OTHER\n'
    assert ('codex', 'AGENTS.md') in unmanaged(kind.USER_INSTRUCTIONS)
    app.delete_runtime('codex')
    app.upsert_runtime(b.BindingRuntimeInstance(
        enabled=False, id='codex', name='codex', type_id='codex-0.159.3',
        gateway_id='default', settings={'agentDir': str(root / 'codex')}))
    assert isinstance(states(docs.id)['codex'], state.NOT_ENABLED), '删除运行时应清理启用记录'
    app.apply_resource_template(docs.id, [target('codex')])
    print('PASS 删除资源与运行时', flush=True)

    # MCP 工作目录与超时按运行时换算；Claude 没有工作目录。
    tuned = app.save_resource_template(resource('tuned', content.MCP(
        transport=b.BindingTemplateMcpTransport.STDIO(command='t', args=[], env={}, cwd='/srv'),
        timeout_seconds=30))).summary
    reports = app.apply_resource_template(tuned.id, [target('claude'), target('codex'), target('pi')])
    assert any('工作目录' in n for n in next(r.notes for r in reports if r.runtime_id == 'claude'))
    claude_json = json.loads((root / 'claude' / '.claude.json').read_text())['mcpServers']['tuned']
    assert claude_json['timeout'] == 30000 and 'cwd' not in claude_json
    codex_toml = (root / 'codex' / 'config.toml').read_text()
    assert 'tool_timeout_sec = 30' in codex_toml and 'cwd = "/srv"' in codex_toml
    pi_json = json.loads((root / 'pi' / 'mcp.json').read_text())['mcpServers']['tuned']
    assert pi_json['timeout'] == 30 and pi_json['cwd'] == '/srv'
    back = app.import_runtime_resource('codex', kind.MCP_SERVERS, 'tuned').template.content
    assert back.timeout_seconds == 30 and back.transport.cwd == '/srv'
    print('PASS MCP 工作目录与超时', flush=True)

    # 差异：原生被改时给出两侧文本。
    (root / 'pi' / 'mcp.json').write_text(json.dumps({'mcpServers': {'tuned': {'command': 'other'}}}))
    drift = app.resource_drift('pi', tuned.id)
    assert len(drift) == 1 and '"other"' in drift[0].native and '"t"' in drift[0].library
    assert 'other' not in repr(drift)
    app.resolve_resource_binding('pi', tuned.id, b.BindingResolution.OVERWRITE)
    assert app.resource_drift('pi', tuned.id) == []
    print('PASS 差异', flush=True)

    # 用原生内容更新资源库：资源取原生内容，原生按资源库格式重写，其他运行时随之同步。
    shared = app.save_resource_template(resource(
        'shared', skill(b'---\nname: shared\ndescription: d\n---\nlibrary\n'))).summary
    app.apply_resource_template(shared.id, [target('claude'), target('codex'), target('pi')])
    shared_file = lambda identity: next((root / identity).rglob('shared/SKILL.md'))
    shared_file('claude').write_bytes(b'---\nname: shared\ndescription: d\n---\nnative edit\n')
    assert isinstance(states(shared.id)['claude'], state.DRIFTED)
    preview = app.resource_native_import('claude', shared.id)
    assert preview.template.id == shared.id and preview.template.name == 'shared'
    assert b'native edit' in preview.template.content.files[0].bytes
    assert b'library' in app.read_resource_template(shared.id).content.files[0].bytes, '预览不保存'
    reject(lambda: app.resource_native_import('claude', review.id))
    app.resolve_resource_binding('claude', shared.id, b.BindingResolution.TAKE_NATIVE)
    assert b'native edit' in app.read_resource_template(shared.id).content.files[0].bytes
    assert all(b'native edit' in shared_file(r).read_bytes() for r in ('claude', 'codex', 'pi'))
    assert all(isinstance(states(shared.id)[r], state.SYNCED) for r in ('claude', 'codex', 'pi'))
    # MCP：原生的命令进入资源，并写入其他运行时；带不进的字段在预览中列出。
    (root / 'pi' / 'mcp.json').write_text(json.dumps({'mcpServers': {'tuned': {'command': 'native-cmd', 'timeout': 30, 'cwd': '/srv', 'unknownField': 1}}}))
    preview = app.resource_native_import('pi', tuned.id)
    print('INFO MCP 预览说明：%r' % (preview.notes,), flush=True)
    assert preview.template.content.transport.command == 'native-cmd'
    app.resolve_resource_binding('pi', tuned.id, b.BindingResolution.TAKE_NATIVE)
    assert json.loads((root / 'claude' / '.claude.json').read_text())['mcpServers']['tuned']['command'] == 'native-cmd'
    assert 'native-cmd' in (root / 'codex' / 'config.toml').read_text()
    assert all(isinstance(states(tuned.id)[r], state.SYNCED) for r in ('claude', 'codex', 'pi'))
    print('PASS 用原生内容更新资源库', flush=True)

    # 忙时保存留作待同步，空闲后任一请求自动补写。
    (root / 'peer.py').write_text(PEER)
    (root / 'project').mkdir(exist_ok=True)
    app.upsert_acp_runtime(b.BindingRuntimeInstance(
        enabled=True, id='live', name='Live', type_id='acp-v1', gateway_id='default',
        settings={'binary': sys.executable}),
        b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),
                                  home=b.BindingRuntimeHome.INHERIT(), remote_resources_directory=None),
        b.BindingAcpLaunch(args=[str(root / 'peer.py')], environment={'FIXTURE_STATE': str(root / 'live')}))
    default = b.BindingModelSelection.RUNTIME_DEFAULT()
    conversation = app.create_conversation_with_selection('live', str(root / 'project'), default).snapshot.conversation.id
    app.send_turn_with_selection('live', conversation, 'live', default, b.BindingComposedTurn(text='CANCEL', effort=None, attachments=[]))
    saved = app.save_resource_template(resource('tuned', content.MCP(
        transport=b.BindingTemplateMcpTransport.STDIO(command='t2', args=[], env={}, cwd=None),
        timeout_seconds=None), tuned.id))
    assert saved.reports == []
    assert isinstance(states(tuned.id)['codex'], state.PENDING)
    reject(lambda: app.apply_resource_template(tuned.id, [target('codex')]))
    app.cancel('live', conversation)
    deadline = time.monotonic() + 5
    while not isinstance(states(tuned.id)['codex'], state.SYNCED):
        assert time.monotonic() < deadline, '空闲后应自动补写'
        time.sleep(.05)
    assert 'command = "t2"' in (root / 'codex' / 'config.toml').read_text()
    print('PASS 忙时待同步、空闲后自动补写', flush=True)

    app.shutdown()
    reopened = type(app).open(b.BindingOptions(
        home_directory=str(root / 'app'), resources_directory=str(root / 'resources')))
    assert {'review', 'docs', 'helper', 'linter'} <= {s.name for s in reopened.list_resource_templates()}
    assert isinstance(next(s.state for s in reopened.resource_sync_status()
                           if s.template_id == docs.id and s.runtime_id == 'claude'), state.SYNCED)
    reopened.shutdown()
    print('PASS 重开保留', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', required=True, type=Path, help='本次构建的 UniFFI 动态库')
    parser.add_argument('--bindings', required=True, type=Path, help='匹配该库的 Python 绑定目录')
    parser.add_argument('--resources', type=Path, help='可选资源目录；默认使用临时空目录')
    parser.add_argument('--python', type=Path, default=Path(sys.executable), help='运行合成 peer 的 Python 3')
    args = parser.parse_args()
    for name in ('library', 'bindings', 'resources', 'python'):
        path = getattr(args, name)
        if path is not None:
            try:
                setattr(args, name, path.resolve(strict=True))
            except OSError:
                parser.error(f'--{name} 路径不存在')
    if not args.library.is_file() or not args.python.is_file():
        parser.error('--library 和 --python 必须是文件')
    if not (args.bindings / 'velune_bindings.py').is_file():
        parser.error('--bindings 目录缺少 velune_bindings.py')
    if args.resources is not None and not args.resources.is_dir():
        parser.error('--resources 必须是目录')
    if os.name != 'posix':
        parser.error('此手动验收目前使用 Unix 文件权限语义')

    environment = dict(os.environ)
    app = None
    with tempfile.TemporaryDirectory(prefix='velune-management-api-') as temporary:
        root = Path(temporary)
        for name in ('home', 'resources', 'project', 'bindings', 'config', 'cache', 'data', 'tmp'):
            (root / name).mkdir()
                # Always create a fresh import directory and link the explicitly selected library.
        shutil.copyfile(args.bindings / 'velune_bindings.py', root / 'bindings' / 'velune_bindings.py')
        library_name = 'libvelune_bindings.dylib' if sys.platform == 'darwin' else 'libvelune_bindings.so'
        (root / 'bindings' / library_name).symlink_to(args.library)
        os.environ.clear()
        os.environ.update(HOME=str(root / 'home'), PATH=os.defpath, TMPDIR=str(root / 'tmp'),
                          XDG_CONFIG_HOME=str(root / 'config'), XDG_CACHE_HOME=str(root / 'cache'),
                          XDG_DATA_HOME=str(root / 'data'), NO_PROXY='127.0.0.1,localhost')
        try:
            spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings' / 'velune_bindings.py')
            bindings = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = bindings
            spec.loader.exec_module(bindings)
            app = bindings.VeluneApplication.open(bindings.BindingOptions(
                home_directory=str(root / 'app'),
                resources_directory=str(args.resources or root / 'resources')))
            exercise(app, bindings, root)
            app.shutdown()
            app = None
            print('PASS 资源模板库与受管同步手动验收完成；未验证原生 Harness 加载', flush=True)
        finally:
            try:
                if app is not None:
                    app.shutdown()
            finally:
                app = None
                gc.collect()
                sys.modules.pop('velune_bindings', None)
                os.environ.clear()
                os.environ.update(environment)



if __name__ == '__main__':
    main()
