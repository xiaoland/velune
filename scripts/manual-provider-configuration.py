#!/usr/bin/env python3
"""Explicit synthetic acceptance of provider-owned configuration and template copies.

Uses only a temporary HOME and synthetic keys. Never a CI/test entry point.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', required=True, type=Path)
    parser.add_argument('--bindings', required=True, type=Path)
    args = parser.parse_args()
    for path in (args.bundle, args.bindings):
        if not path.is_absolute() or not path.exists():
            parser.error('paths must be absolute and exist')
    environment = dict(os.environ)
    with tempfile.TemporaryDirectory(prefix='velune-provider-configuration-') as directory:
        root = Path(directory)
        (root / 'bindings').mkdir()
        shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
        library = args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib'
        (root / 'bindings' / library.name).symlink_to(library)
        os.environ.clear()
        os.environ.update(HOME=str(root), PATH='/usr/bin:/bin')
        spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        application = None
        try:
            home = root / 'home'
            home.mkdir()
            config = home / 'generic-config.json'
            config.write_text(json.dumps({'schemaVersion': 4, 'obsolete': 'discard'}))
            original = root / 'original-harness.json'
            original.write_text('synthetic original unchanged')
            options = b.BindingOptions(home_directory=str(home),
                resources_directory=str(args.bundle / 'Contents/Resources'))
            application = b.VeluneApplication.open(options)
            reset = json.loads(config.read_text())
            assert reset['schemaVersion'] == 5 and not reset['gateways']
            assert not any('backup' in p.name or p.suffix == '.bak' for p in home.iterdir())
            runtime_home = root / 'synthetic-pi'
            runtime_home.mkdir()
            application.upsert_runtime(b.BindingRuntimeInstance(id='inactive', name='Synthetic inactive runtime',
                type_id='pi', gateway_id='default', model_record_key=None,
                settings={'agentDir': str(runtime_home), 'nodeBinary': '/usr/bin/false'}))
            assert application.list().gateways[0].id == 'default'
            protocol = b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1


            def model(record='', identity='org/model:版本-1'):
                return b.BindingProviderModel(record_key=record, provider_model_id=identity,
                    nickname='Synthetic model', icon=None, context_window=None, max_output_tokens=None,
                    reasoning_levels=None, adapter_metadata_json=None)

            def draft(identity='a', models=None, endpoint='https://synthetic.invalid/v1'):
                return b.BindingProviderDraft(id=identity, name=identity, protocol=protocol,
                    endpoint=endpoint, models=models or [model()])

            def rejected(operation):
                before = config.read_bytes()
                try:
                    operation()
                except b.BindingError:
                    assert config.read_bytes() == before
                    return
                raise AssertionError('invalid mutation succeeded')

            key_one = 'SYNTHETIC_KEY_ONE'
            key_two = 'SYNTHETIC_KEY_TWO'
            application.save_provider('default', draft(), b.BindingAuthenticationEdit.SET_API_KEY(value=key_one))
            snapshot = application.list()
            provider = snapshot.gateways[0].providers[0]
            record = provider.models[0].record_key
            assert record and record != provider.models[0].provider_model_id
            assert key_one not in repr(snapshot)
            assert application.read_provider_api_key('default', 'a') == key_one
            assert stat.S_IMODE(config.stat().st_mode) == 0o600
            assert key_one in config.read_text()
            edited = draft(models=[model(record, 'changed/model-id')], endpoint='https://changed.invalid/v1')
            application.save_provider('default', edited, b.BindingAuthenticationEdit.KEEP())
            assert application.read_provider_api_key('default', 'a') == key_one
            saved = application.list().gateways[0].providers[0]
            assert saved.endpoint == 'https://changed.invalid/v1'
            assert saved.models[0].record_key == record and saved.models[0].provider_model_id == 'changed/model-id'
            edited.protocol = b.BindingGatewayProtocol.RESPONSES_V1
            application.save_provider('default', edited, b.BindingAuthenticationEdit.SET_API_KEY(value=key_two))
            assert application.read_provider_api_key('default', 'a') == key_two
            rejected(lambda: application.save_provider('default', edited,
                b.BindingAuthenticationEdit.SET_API_KEY(value='invalid\nkey')))

            template = b.BindingModelTemplate(id='', name='Reusable', suggested_provider_model_id='suggested-id',
                nickname='Template model', icon=None, context_window=32768, max_output_tokens=4096,
                reasoning_levels=['low', 'high'])
            templates = application.save_model_template(template)
            template = next(item for item in templates if item.name == 'Reusable')
            copied = model(identity=template.suggested_provider_model_id)
            copied.nickname = template.nickname
            copied.context_window = template.context_window
            copied.max_output_tokens = template.max_output_tokens
            copied.reasoning_levels = list(template.reasoning_levels)
            application.save_provider('default', draft('b', [copied]), b.BindingAuthenticationEdit.CLEAR())
            template.context_window = 65536
            application.save_model_template(template)
            second = next(item for item in application.list().gateways[0].providers if item.id == 'b')
            assert second.models[0].context_window == 32768
            assert second.models[0].record_key != record
            assert second.models[0].adapter_metadata_json is None
            application.shutdown()
            application = b.VeluneApplication.open(options)
            assert application.read_provider_api_key('default', 'a') == key_two
            assert key_two not in repr(application.list())
            application.delete_model_template(template.id)
            assert next(item for item in application.list().gateways[0].providers if item.id == 'b').models
            application.save_provider('default', edited, b.BindingAuthenticationEdit.CLEAR())
            rejected(lambda: application.read_provider_api_key('default', 'a'))
            application.delete_provider('default', 'b')
            assert len(application.list().gateways[0].providers) == 1
            application.shutdown()
            application = None
            assert original.read_text() == 'synthetic original unchanged'
            for label, contents in [('future', json.dumps({'schemaVersion': 6})),
                                    ('malformed', '{broken'),
                                    ('old-fields', json.dumps({**reset, 'authenticationBindings': []}))]:
                invalid_home = root / label
                invalid_home.mkdir()
                invalid_config = invalid_home / 'generic-config.json'
                invalid_config.write_text(contents)
                before = invalid_config.read_bytes()
                try:
                    application = b.VeluneApplication.open(b.BindingOptions(
                        home_directory=str(invalid_home), resources_directory=str(args.bundle / 'Contents/Resources')))
                except b.BindingError:
                    assert invalid_config.read_bytes() == before
                else:
                    raise AssertionError(label + ' config accepted')
            for log in (home / 'logs').rglob('*') if (home / 'logs').exists() else []:
                if log.is_file():
                    text = log.read_text()
                    assert key_one not in text and key_two not in text
            print(json.dumps({'acceptance': 'PASSED', 'schemaFiveHardCutoff': True,
                'providerOwnedModelsAndAuthentication': True, 'runtimeCanPrecedeProvider': True, 'editableIdProtocolEndpoint': True,
                'stableHiddenRecordKey': True, 'explicitKeyReadEditReopen': True,
                'keyAbsentFromListAndLogs': True, 'filePermissions0600': True,
                'templateCopiesDoNotPropagateOrCarryProjection': True,
                'noBackupOrOriginalHarnessMutation': True, 'invalidConfigLeavesFileUnchanged': True}))
        finally:
            if application:
                application.shutdown()
            os.environ.clear()
            os.environ.update(environment)


if __name__ == '__main__':
    main()
