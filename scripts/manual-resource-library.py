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


def exercise(app, b, root):
    kind = b.BindingRuntimeResourceKind
    scope = b.BindingRuntimeResourceScope
    revision = b.BindingRuntimeResourceRevision
    content = b.BindingResourceTemplateContent
    outcome = b.BindingTemplateApplyOutcome
    state = b.BindingTemplateSyncState

    for identity, variant in [('claude', 'claude-code-2.1.286'), ('codex', 'codex-0.159.3'),
                              ('pi', 'pi-1.0.2')]:
        app.upsert_runtime(b.BindingRuntimeInstance(
            enabled=False, id=identity, name=identity, type_id=variant,
            gateway_id='default', settings={'agentDir': str(root / identity)}))

    def template(name, body, identity=''):
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

    def report(reports, identity):
        return next(r.outcome for r in reports if r.runtime_id == identity)

    def native(identity, path):
        return root / identity / path

    # 模板：摘要不含内容；名称与同类唯一；类别不可改。
    summary = app.save_resource_template(template(
        'review', skill(b'---\nname: review\ndescription: d\n---\nv1\n', [('notes/a.md', b'A')])))
    assert summary.id and summary.kind == kind.SKILLS
    reject(lambda: app.save_resource_template(template('review', skill(b'---\nname: review\ndescription: d\n---\n'))))
    reject(lambda: app.save_resource_template(template('bad name', skill(b'x'))))
    reject(lambda: app.save_resource_template(template('nofile', content.SKILL(files=[]))))
    reject(lambda: app.save_resource_template(template(
        'review', content.INSTRUCTIONS(body='x'), summary.id)))
    assert [s.name for s in app.list_resource_templates()] == ['review']
    print('PASS 模板保存与校验', flush=True)

    # 首次应用：预览不写入；应用创建并受管。
    reports = app.preview_resource_template(summary.id, [target('claude'), target('codex')])
    assert isinstance(report(reports, 'claude'), outcome.CREATE)
    assert not (root / 'claude').exists()
    reports = app.apply_resource_template(summary.id, [target('claude'), target('codex')])
    assert isinstance(report(reports, 'claude'), outcome.CREATE)
    assert native('claude', 'skills/review/SKILL.md').is_file() or any((root / 'claude').rglob('SKILL.md'))
    assert all(isinstance(s.state, state.SYNCED) for s in app.runtime_template_status('claude'))
    reports = app.apply_resource_template(summary.id, [target('claude')])
    assert isinstance(report(reports, 'claude'), outcome.UNCHANGED)
    print('PASS 应用、预览、幂等', flush=True)

    # 模板更新 → TemplateUpdated → 同步；删去文件会同步删除。
    app.save_resource_template(template(
        'review', skill(b'---\nname: review\ndescription: d\n---\nv2\n'), summary.id))
    assert isinstance(app.runtime_template_status('claude')[0].state, state.TEMPLATE_UPDATED)
    reports = app.apply_resource_template(summary.id, [target('claude')])
    assert isinstance(report(reports, 'claude'), outcome.UPDATE)
    skill_root = next((root / 'claude').rglob('SKILL.md')).parent
    assert not (skill_root / 'notes' / 'a.md').exists()
    assert b'v2' in (skill_root / 'SKILL.md').read_bytes()
    assert isinstance(app.runtime_template_status('claude')[0].state, state.SYNCED)
    print('PASS 模板更新同步、移除的文件被清理', flush=True)

    # 原生侧修改 → Drifted；应用报 Conflict；三种处置。
    (skill_root / 'SKILL.md').write_bytes(b'---\nname: review\ndescription: d\n---\nhand edit\n')
    assert isinstance(app.runtime_template_status('claude')[0].state, state.DRIFTED)
    reports = app.apply_resource_template(summary.id, [target('claude')])
    assert isinstance(report(reports, 'claude'), outcome.CONFLICT)
    assert b'hand edit' in (skill_root / 'SKILL.md').read_bytes()
    app.resolve_resource_binding('claude', summary.id, b.BindingResolution.OVERWRITE)
    assert b'v2' in (skill_root / 'SKILL.md').read_bytes()
    assert isinstance(app.runtime_template_status('claude')[0].state, state.SYNCED)
    (skill_root / 'SKILL.md').write_bytes(b'---\nname: review\ndescription: d\n---\nkeep\n')
    app.resolve_resource_binding('claude', summary.id, b.BindingResolution.UNMANAGE)
    assert app.runtime_template_status('claude') == []
    assert b'keep' in (skill_root / 'SKILL.md').read_bytes()
    # 解除后原生条目成为未受管：应用冲突，覆盖后重新受管。
    reports = app.apply_resource_template(summary.id, [target('claude')])
    assert isinstance(report(reports, 'claude'), outcome.CONFLICT)
    reports = app.apply_resource_template(summary.id, [target('claude', True)])
    assert isinstance(report(reports, 'claude'), outcome.UPDATE)
    # 被外部删除 → Missing。
    (skill_root / 'SKILL.md').unlink()
    assert isinstance(app.runtime_template_status('claude')[0].state, state.MISSING)
    app.resolve_resource_binding('claude', summary.id, b.BindingResolution.REMOVE)
    assert app.runtime_template_status('claude') == [] and not skill_root.exists()
    print('PASS 漂移、冲突、覆盖／解除受管／移除', flush=True)

    # 指令整文件、MCP 秘密、子代理、跨运行时跳过原因。
    instructions = app.save_resource_template(template('global', content.INSTRUCTIONS(body='RULES\n')))
    mcp = app.save_resource_template(template('docs', content.MCP(
        transport=b.BindingTemplateMcpTransport.STDIO(
            command='docs-mcp', args=['--x'], env={'TOKEN': 'SYNTHETIC-SECRET'}))))
    agent = app.save_resource_template(template('helper', content.SUBAGENT(
        description='d', instructions='do it', model=None, tools=[])))
    assert 'SYNTHETIC-SECRET' not in repr(app.read_resource_template(mcp.id))
    assert 'SYNTHETIC-SECRET' not in repr(app.list_resource_templates())
    stored = (root / 'app' / 'resource-templates.json')
    assert oct(stored.stat().st_mode & 0o777) == '0o600'
    for item in (instructions, mcp, agent):
        reports = app.apply_resource_template(item.id, [target('claude'), target('codex'), target('pi')])
        assert not any(isinstance(r.outcome, outcome.FAILED) for r in reports)
    assert (root / 'claude' / 'CLAUDE.md').read_text() == 'RULES\n'
    assert (root / 'codex' / 'AGENTS.md').read_text() == 'RULES\n'
    print('PASS 指令、MCP、子代理模板', flush=True)

    # 删除模板解除受管但不删原生条目；删除运行时清理绑定。
    app.delete_resource_template(instructions.id)
    assert (root / 'claude' / 'CLAUDE.md').exists()
    assert all(s.template_id != instructions.id for s in app.runtime_template_status('claude'))
    app.delete_runtime('codex')
    app.upsert_runtime(b.BindingRuntimeInstance(
        enabled=False, id='codex', name='codex', type_id='codex-0.159.3',
        gateway_id='default', settings={'agentDir': str(root / 'codex')}))
    assert app.runtime_template_status('codex') == [], '删除运行时应清理绑定'
    print('PASS 模板删除', flush=True)

    # 持久化：重开后绑定仍在。
    app.shutdown()
    reopened = type(app).open(b.BindingOptions(
        home_directory=str(root / 'app'), resources_directory=str(root / 'resources')))
    assert {s.name for s in reopened.list_resource_templates()} == {'review', 'docs', 'helper'}
    assert reopened.runtime_template_status('claude')
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
