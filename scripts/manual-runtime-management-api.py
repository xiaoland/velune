#!/usr/bin/env python3
"""手动验收资源管理的 UniFFI 契约与生命周期；不接入 CI。

只启动内置合成 ACP peer，不启动原生 Harness 或调用模型。原生资源的实际加载
由 manual-runtime-management.py 等独立验收覆盖。所有配置、资源与 HOME 都在临时目录。
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


def exercise(app, b, root, python):
    kind = b.BindingRuntimeResourceKind
    scope = b.BindingRuntimeResourceScope
    revision = b.BindingRuntimeResourceRevision

    def key(category, path, resource_scope=scope.USER):
        return b.BindingRuntimeResourceKey(kind=category, scope=resource_scope, path=path)

    def reject(operation, code=None):
        try:
            operation()
        except b.BindingError.Diagnostic as error:
            if code is not None:
                assert error.code == code, (error.code, str(error))
                assert error.phase == 'runtime_management'
                assert error.kind == b.BindingFailureKind.RUNTIME_MANAGEMENT
            return
        raise AssertionError('操作应被拒绝')

    def write(identity, resource, data, previous=None, executable=None, cwd=None):
        expected = revision.MISSING() if previous is None else revision.MATCH(previous)
        return app.write_runtime_resource(identity, cwd, resource, expected, data, executable)

    variants = [('codex', 'codex-0.159.3'), ('claude', 'claude-code-2.1.286'),
                ('pi', 'pi-1.0.2'), ('dsh', 'dsh-acp-0.2.0-rc.2'), ('generic', 'acp-v1')]
    for identity, variant in variants:
        # Disabled instances deliberately have no executable or Node path.
        settings = {} if identity == 'generic' else {'agentDir': str(root / identity)}
        app.upsert_runtime(b.BindingRuntimeInstance(
            enabled=False, id=identity, name=identity, type_id=variant,
            gateway_id='default', settings=settings))

    for identity in ('codex', 'claude', 'pi', 'dsh'):
        catalog = app.runtime_resource_catalog(identity, None, kind.USER_INSTRUCTIONS, scope.USER)
        assert isinstance(catalog.support, b.BindingRuntimeResourceSupport.SUPPORTED)
        assert catalog.entries and isinstance(catalog.entries[0].state, b.BindingRuntimeResourceState.MISSING)
        assert not (root / identity).exists(), 'catalog 不应创建资源根目录'
        resource = catalog.entries[0].key
        document = write(identity, resource, b'SYNTHETIC INSTRUCTIONS\n')
        loaded = app.read_runtime_resource(identity, None, resource)
        assert loaded.bytes == b'SYNTHETIC INSTRUCTIONS\n'
        assert loaded.entry.revision == document.entry.revision
        updated = write(identity, resource, b'EDITED INSTRUCTIONS\n', document.entry.revision)
        reject(lambda: write(identity, resource, b'STALE', document.entry.revision), 'conflict')
        app.delete_runtime_resource(identity, None, resource, revision.MATCH(updated.entry.revision))
        reject(lambda: app.read_runtime_resource(identity, None, resource), 'missing')

    for category in (kind.USER_INSTRUCTIONS, kind.MCP_SERVERS, kind.SKILLS, kind.NATIVE_AGENT_DEFINITIONS):
        catalog = app.runtime_resource_catalog('generic', None, category, scope.USER)
        assert isinstance(catalog.support, b.BindingRuntimeResourceSupport.UNSUPPORTED)
    for identity, category in [('pi', kind.NATIVE_AGENT_DEFINITIONS), ('dsh', kind.MCP_SERVERS)]:
        catalog = app.runtime_resource_catalog(identity, None, category, scope.USER)
        assert isinstance(catalog.support, b.BindingRuntimeResourceSupport.UNSUPPORTED)
    print('PASS 停用实例、Missing／Unsupported、instructions CRUD、conflict 诊断保真', flush=True)

    app.upsert_runtime(b.BindingRuntimeInstance(
        enabled=False, id='project-only', name='Project only', type_id='codex-0.159.3',
        gateway_id='default', settings={}))
    project = str(root / 'project')
    catalog = app.runtime_resource_catalog('project-only', project, kind.USER_INSTRUCTIONS, scope.PROJECT)
    document = write('project-only', catalog.entries[0].key, b'PROJECT ONLY\n', cwd=project)
    app.delete_runtime_resource('project-only', project, document.entry.key, revision.MATCH(document.entry.revision))
    definitions = [
        ('codex', 'fixture.toml', b'name = "fixture"\ndescription = "Synthetic fixture"\ndeveloper_instructions = "Read only fixture"\n'),
        ('claude', 'fixture.md', b'---\nname: fixture\ndescription: Synthetic fixture\n---\nRead only fixture.\n'),
    ]
    for identity, path, data in definitions:
        document = write(identity, key(kind.NATIVE_AGENT_DEFINITIONS, path), data)
        assert app.read_runtime_resource(identity, None, document.entry.key).bytes == data
        app.delete_runtime_resource(identity, None, document.entry.key, revision.MATCH(document.entry.revision))
    print('PASS 无 user root 的 Project 范围、原生 Agent 定义 bytes', flush=True)

    manifest = b'---\nname: fixture\ndescription: Synthetic fixture\n---\nFixture instructions.\n'
    skill = write('codex', key(kind.SKILLS, 'fixture/SKILL.md'), manifest)
    asset_key = key(kind.SKILLS, 'fixture/assets/data.bin')
    payload = bytes([0, 255, 4, 8, 42])
    asset = write('codex', asset_key, payload)
    assert app.read_runtime_resource('codex', None, asset_key).bytes == payload
    executable = write('codex', key(kind.SKILLS, 'fixture/scripts/helper.sh'), b'#!/bin/sh\nexit 0\n', executable=True)
    assert executable.executable
    catalog = app.runtime_resource_catalog('codex', None, kind.SKILLS, scope.USER)
    assert {'fixture/SKILL.md', 'fixture/assets/data.bin', 'fixture/scripts/helper.sh'} <= {entry.key.path for entry in catalog.entries}
    bundle = next(entry for entry in catalog.entries if entry.key.path == 'fixture')
    reject(lambda: app.delete_runtime_resource('codex', None, bundle.key, revision.MATCH(bundle.revision)))
    reject(lambda: write('codex', key(kind.SKILLS, '../escape'), b'x'), 'invalid')
    for document in (skill, asset, executable):
        app.delete_runtime_resource('codex', None, document.entry.key, revision.MATCH(document.entry.revision))
    for relative in ('fixture/assets', 'fixture/scripts', 'fixture'):
        entries = app.runtime_resource_catalog('codex', None, kind.SKILLS, scope.USER).entries
        entry = next(item for item in entries if item.key.path == relative)
        app.delete_runtime_resource('codex', None, entry.key, revision.MATCH(entry.revision))
    print('PASS skills 二进制资产、可执行属性、递归目录、路径限制及逐项删除', flush=True)

    for identity in ('codex', 'claude', 'pi'):
        resource = key(kind.MCP_SERVERS, 'fixture.server')
        # This declaration is only read and written; no MCP command is executed.
        data = json.dumps({'command': str(python), 'args': ['--version'],
                           'env': {'SYNTHETIC_VALUE': 'not-a-real-secret'}}).encode()
        document = write(identity, resource, data)
        assert json.loads(app.read_runtime_resource(identity, None, resource).bytes)['command'] == str(python)
        app.delete_runtime_resource(identity, None, resource, revision.MATCH(document.entry.revision))
    print('PASS MCP 单条原生 JSON、opaque server name、删除', flush=True)

    execution = b.BindingRuntimeExecution(
        transport=b.BindingRuntimeTransport.LOCAL(),
        home=b.BindingRuntimeHome.MANAGED(remote_root=None), remote_resources_directory=None)
    app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
        enabled=False, id='managed', name='Managed', type_id='claude-code-2.1.286',
        gateway_id='default', settings={}), execution)
    managed = app.runtime_resource_catalog('managed', None, kind.USER_INSTRUCTIONS, scope.USER).entries[0]
    assert not Path(managed.absolute_path).parent.exists()
    assert Path(managed.absolute_path).is_relative_to(root / 'app' / 'runtime-homes')
    write('managed', managed.key, b'MANAGED HOME INSTRUCTIONS\n')
    assert Path(managed.absolute_path).read_bytes() == b'MANAGED HOME INSTRUCTIONS\n'
    print('PASS managed HOME 稳定定位、查询不创建目录', flush=True)

    execution = b.BindingRuntimeExecution(
        transport=b.BindingRuntimeTransport.LOCAL(), home=b.BindingRuntimeHome.INHERIT(),
        remote_resources_directory=None)
    app.upsert_acp_runtime(b.BindingRuntimeInstance(
        enabled=True, id='live', name='Live', type_id='acp-v1', gateway_id='default',
        settings={'binary': str(python)}), execution,
        b.BindingAcpLaunch(args=[str(root / 'peer.py')], environment={'FIXTURE_STATE': str(root / 'live')}))
    default = b.BindingModelSelection.RUNTIME_DEFAULT()
    snapshot = app.create_conversation_with_selection('live', project, default).snapshot
    conversation = snapshot.conversation.id
    change = key(kind.USER_INSTRUCTIONS, 'AGENTS.md')
    write('codex', change, b'IDLE RESOURCE EDIT\n')
    assert app.snapshot('live', conversation).snapshot.actions.can_send, '资源编辑不应替换空闲执行器'
    app.send_turn_with_selection('live', conversation, 'live', default, 'CANCEL')
    app.runtime_resource_catalog('codex', None, kind.USER_INSTRUCTIONS, scope.USER)
    reject(lambda: write('codex', change, b'MUST_REJECT'))
    app.cancel('live')
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if app.snapshot('live', conversation).snapshot.actions.can_send:
            break
        time.sleep(.02)
    else:
        raise AssertionError('合成 peer 取消未完成')
    events = [json.loads(line) for line in (root / 'live' / 'events.jsonl').read_text().splitlines()]
    assert sum(event['kind'] == 'start' for event in events) == 1
    assert sum(event['kind'] == 'prompt' for event in events) == 1
    targets = [b.BindingCollaborationTarget(
        id='live', name='Live', runtime_id='live', cwd=project, model_selection=default)]
    app.create_collaborative_conversation('live', project, default, targets,
                                         b.BindingCollaborationLimits(max_children=1, turn_timeout_ms=5000))
    app.runtime_resource_catalog('codex', None, kind.USER_INSTRUCTIONS, scope.USER)
    reject(lambda: write('codex', change, b'MUST_REJECT_SCOPE'))
    app.close_collaboration()
    print('PASS busy／scope 写入门禁、只读可用、空闲合成 ACP 进程保持', flush=True)


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
        (root / 'peer.py').write_text(PEER)
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
            exercise(app, bindings, root, args.python)
            app.shutdown()
            app = None
            print('PASS 资源管理 API 手动验收完成；合成 peer 不代表原生 Harness 加载验收', flush=True)
        finally:
            try:
                if app is not None:
                    try:
                        app.cancel('live')
                    except bindings.BindingError:
                        pass
                    deadline = time.monotonic() + 5
                    while True:
                        try:
                            app.close_collaboration()
                            app.shutdown()
                            break
                        except bindings.BindingError:
                            if time.monotonic() >= deadline:
                                raise
                            time.sleep(.02)
            finally:
                app = None
                gc.collect()
                sys.modules.pop('velune_bindings', None)
                os.environ.clear()
                os.environ.update(environment)



if __name__ == '__main__':
    main()
