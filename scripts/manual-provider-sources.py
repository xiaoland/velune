#!/usr/bin/env python3
"""Provider sources acceptance by hand; never a test or CI entry point.

Uses a temporary HOME, synthetic Codex config.toml and Claude Code
settings.json with fixture keys, and a fake `claude` that answers
`auth status` from a fixture account file. Checks importing Codex and
Claude Code runtime providers, referencing an existing Claude Code login
(user default and runtime) without copying it or ever signing it out, one
account per provider, and stale preview rejection. It does not prove real
Claude Code or Codex behavior.
Run: python3 scripts/manual-provider-sources.py --bundle /abs/Velune.app --bindings /abs/python-bindings
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile

FAKE_CLAUDE = r'''#!__PYTHON__
import json, os, sys
from pathlib import Path
home = Path(os.environ['HOME'])
directory = Path(os.environ.get('CLAUDE_CONFIG_DIR') or home / '.claude')
with open(__LOG__, 'a') as log:
    log.write(json.dumps({'args': sys.argv[1:], 'configDir': os.environ.get('CLAUDE_CONFIG_DIR')}) + '\n')
if sys.argv[1:] == ['--version']:
    print('2.1.286 (Claude Code)')
elif sys.argv[1:3] == ['auth', 'status']:
    account = (directory / 'fixture-account').read_text().strip() if (directory / 'fixture-account').exists() else None
    print(json.dumps({'loggedIn': account is not None, 'authMethod': 'claude.ai' if account else 'none',
                      'accountUuid': account, 'configDirectory': str(directory)}))
else:
    sys.exit(3)
'''

CODEX_CONFIG = '''model = "relay-gpt"
model_provider = "relay"

[model_providers.relay]
name = "Relay"
base_url = "https://codex-relay.example.test/v1"
experimental_bearer_token = "fixture-codex-token"

[model_providers.envy]
name = "Envy"
base_url = "https://envy.example.test/v1"
env_key = "ENVY_KEY"
http_headers = { "X-Fixture" = "1" }

[model_providers.chat]
base_url = "https://chat.example.test/v1"
wire_api = "chat"

[profiles.fast]
model_provider = "envy"
model = "envy-fast"
'''
SECRETS = ['fixture-codex-token', 'fixture-claude-key', 'fixture-bearer-token']


def check(ok, message):
    if not ok:
        raise AssertionError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--bindings', type=Path, required=True)
    args = parser.parse_args()
    library = args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib'
    for path in (library, args.bindings):
        if not path.is_absolute() or not path.exists():
            parser.error('absolute existing paths required')
    with tempfile.TemporaryDirectory(prefix='velune-provider-sources-') as directory:
        root = Path(directory)
        for name in ('bindings', 'home/.claude', 'claude-a', 'claude-b', 'codex', 'resources'):
            (root / name).mkdir(parents=True)
        shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
        (root / 'bindings' / library.name).symlink_to(library)
        log = root / 'claude-calls.jsonl'
        claude = root / 'claude'
        claude.write_text(FAKE_CLAUDE.replace('__PYTHON__', sys.executable).replace('__LOG__', repr(str(log))))
        claude.chmod(0o700)
        (root / 'home/.claude/fixture-account').write_text('acct-default')
        (root / 'home/.claude/settings.json').write_text(json.dumps({'env': {
            'ANTHROPIC_BASE_URL': 'https://relay.example.test', 'ANTHROPIC_AUTH_TOKEN': 'fixture-bearer-token',
            'ANTHROPIC_MODEL': 'relay-model-1'}}))
        (root / 'claude-a/fixture-account').write_text('acct-a')
        (root / 'claude-a/settings.json').write_text(json.dumps({'model': 'claude-fixture-model', 'env': {'ANTHROPIC_API_KEY': 'fixture-claude-key'}}))
        (root / 'claude-b/fixture-account').write_text('acct-b')
        (root / 'codex/config.toml').write_text(CODEX_CONFIG)
        sources = [root / 'codex/config.toml', root / 'claude-a/settings.json', root / 'home/.claude/settings.json']
        before = {path: digest(path) for path in sources}
        os.environ.clear()
        os.environ.update(HOME=str(root / 'home'), PATH='/usr/bin:/bin')
        spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        velune_home = root / 'velune'
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(velune_home), resources_directory=str(root / 'resources')))
        try:
            def runtime(id, type_id, agent_dir):
                app.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id=id, name=id, type_id=type_id, gateway_id='default',
                    settings={'binary': str(claude), 'agentDir': str(agent_dir), 'nodeBinary': sys.executable}))
            runtime('codex', 'codex-0.159', root / 'codex')
            runtime('claude', 'claude-code-2.1', root / 'claude-a')
            runtime('claude-b', 'claude-code-2.1', root / 'claude-b')
            types = {t.id for t in app.list().provider_import_types}
            check({'pi-1.0', 'codex-0.159', 'claude-code-2.1'} <= types, types)

            def source(family, instance):
                return b.BindingProviderImportSource(kind='harness', harness_type_id=family, source_instance_id=instance, provider_id=None, settings={})

            def providers():
                return {p.name: p for p in next(g for g in app.list().gateways if g.id == 'default').providers}

            # Codex: literal bearer copied, env key left to fill, unsupported wire API listed as a warning.
            preview = app.preview_provider_import('default', source('codex', 'codex'))
            check(not any(secret in repr(preview) for secret in SECRETS), 'preview exposes a secret')
            listed = {p.source_provider_id: p for p in preview.providers}
            check(set(listed) == {'relay', 'envy'}, listed.keys())
            check([m.provider_model_id for m in listed['relay'].models] == ['relay-gpt'], listed['relay'].models)
            check([m.provider_model_id for m in listed['envy'].models] == ['envy-fast'], listed['envy'].models)
            check(any('ENVY_KEY' in issue for issue in listed['envy'].issues) and any('请求头' in issue for issue in listed['envy'].issues), listed['envy'].issues)
            check(any('chat' in warning for warning in preview.warnings) and not any('OpenAI' in w for w in preview.warnings), preview.warnings)
            check(listed['relay'].protocol == 'responsesV1' and listed['relay'].endpoint == 'https://codex-relay.example.test/v1', listed['relay'])
            selections = [b.BindingImportSelection(provider_id=p.id, candidate_keys=[m.candidate_key for m in p.models]) for p in listed.values()]
            app.apply_provider_import('default', source('codex', 'codex'), preview.token, selections, False)
            saved = providers()
            check(app.read_provider_api_key('default', saved['Relay'].id) == 'fixture-codex-token', 'relay key not copied')
            check(saved['Envy'].authentication.method != b.BindingAuthenticationMethod.API_KEY or not saved['Envy'].authentication.configured, saved['Envy'].authentication)
            # A changed source invalidates the shown preview.
            (root / 'codex/config.toml').write_text(CODEX_CONFIG + '\n# edited\n')
            try:
                app.apply_provider_import('default', source('codex', 'codex'), preview.token, selections, True)
                check(False, 'stale preview applied')
            except b.BindingError as error:
                check('changed' in str(error), error)
            (root / 'codex/config.toml').write_text(CODEX_CONFIG)

            # Claude Code runtime: its login referenced, its settings API key copied.
            preview = app.preview_provider_import('default', source('claude-code', 'claude'))
            check(not any(secret in repr(preview) for secret in SECRETS), 'preview exposes a secret')
            listed = {p.source_provider_id: p for p in preview.providers}
            check(set(listed) == {'claudeCodeLogin', 'claudeCodeSettingsApi'}, listed.keys())
            check(listed['claudeCodeLogin'].endpoint is None and listed['claudeCodeLogin'].can_import, listed['claudeCodeLogin'])
            check(listed['claudeCodeSettingsApi'].endpoint == 'https://api.anthropic.com', listed['claudeCodeSettingsApi'])
            login = listed['claudeCodeLogin']
            app.apply_provider_import('default', source('claude-code', 'claude'), preview.token, [
                b.BindingImportSelection(provider_id=login.id, candidate_keys=[m.candidate_key for m in login.models if m.provider_model_id in ('sonnet', 'opus')]),
                b.BindingImportSelection(provider_id=listed['claudeCodeSettingsApi'].id, candidate_keys=[m.candidate_key for m in listed['claudeCodeSettingsApi'].models])], False)
            saved = providers()
            linked = next(p for p in saved.values() if p.id == login.id)
            check(linked.authentication.configured and linked.authentication.linked_login == b.BindingLinkedClaudeLogin.RUNTIME(instance_id='claude'), linked.authentication)
            check(not any(a.id in ('login', 'logout') for a in linked.authentication.actions), linked.authentication.actions)
            check([m.provider_model_id for m in linked.models] == ['sonnet', 'opus'], linked.models)
            api = next(p for p in saved.values() if p.id == listed['claudeCodeSettingsApi'].id)
            check(app.read_provider_api_key('default', api.id) == 'fixture-claude-key', 'claude key not copied')

            # The user's default login, referenced from the Claude subscription composition.
            draft = b.BindingProviderDraft(id='', name='Default Claude', protocol=b.BindingGatewayProtocol.MESSAGES_V1, endpoint='',
                execution_backend=b.BindingExecutionBackend.CLAUDE_CLI(binary=str(claude)), request_profile=b.BindingRequestProfile.NATIVE,
                authentication_source=b.BindingProviderAuthenticationSource.CLAUDE_CLI, quota_source=b.BindingProviderQuotaSource.CLAUDE_CLI_USAGE,
                models=[b.BindingProviderModel(record_key='', provider_model_id='sonnet', nickname='', icon=None, context_window=None, max_output_tokens=None, reasoning_levels=None)], quota_limits=[])
            app.save_provider('default', draft, b.BindingAuthenticationEdit.USE_CLAUDE_CODE_LOGIN(login=b.BindingLinkedClaudeLogin.USER_DEFAULT()))
            default = providers()['Default Claude']
            check(default.authentication.configured and default.authentication.linked_login == b.BindingLinkedClaudeLogin.USER_DEFAULT(), default.authentication)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            check(any(c['args'][:2] == ['auth', 'status'] and c['configDir'] is None for c in calls), 'user default status ran with CLAUDE_CONFIG_DIR')

            # One account, one provider: the same account behind another login is refused.
            draft.name = 'Second Claude'
            try:
                app.save_provider('default', draft, b.BindingAuthenticationEdit.USE_CLAUDE_CODE_LOGIN(login=b.BindingLinkedClaudeLogin.RUNTIME(instance_id='claude')))
                check(False, 'duplicate account accepted')
            except b.BindingError as error:
                check('已绑定另一个提供商' in str(error), error)
            (root / 'claude-b/fixture-account').write_text('acct-default')
            try:
                app.save_provider('default', draft, b.BindingAuthenticationEdit.USE_CLAUDE_CODE_LOGIN(login=b.BindingLinkedClaudeLogin.RUNTIME(instance_id='claude-b')))
                check(False, 'duplicate account accepted')
            except b.BindingError as error:
                check('已绑定另一个提供商' in str(error), error)
            # A lapsed referenced login reads as not signed in after inspection.
            (root / 'claude-a/fixture-account').unlink()
            check(not app.authentication_inspect('default', linked.id).configured, 'lapsed login still configured')
            check(not providers()[linked.name].authentication.configured, 'lapsed login persisted as configured')
            # Removing referencing providers signs nothing out and copies nothing.
            app.delete_provider('default', linked.id)
            app.delete_provider('default', default.id)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            check(not any('logout' in c['args'] or 'login' in c['args'] for c in calls), calls)
            check(not (velune_home / 'claude-cli').exists() or not any((velune_home / 'claude-cli').iterdir()), 'a login directory was created')
            check(all(digest(path) == before[path] for path in sources), 'source files changed')
            logs = ''.join(p.read_text(errors='replace') for p in (velune_home / 'logs').glob('*')) if (velune_home / 'logs').exists() else ''
            check(not any(secret in logs for secret in SECRETS), 'a secret reached the logs')
            print(json.dumps({'codexImport': True, 'claudeImport': True, 'linkedLogins': True, 'duplicateRefused': True,
                              'staleRefused': True, 'noLogout': True, 'secretsKeptOutOfLogs': True}))
        finally:
            app.shutdown()


if __name__ == '__main__':
    main()
