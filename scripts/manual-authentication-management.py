#!/usr/bin/env python3
"""Explicit synthetic UniFFI acceptance of the central authentication repository.

All files live in a temporary home. The credential helper fails if invoked:
configuration reset and registry operations must not read any secret or source file.
This is a manual aid, never a CI or automated test entry point.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
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
    with tempfile.TemporaryDirectory(prefix='velune-central-auth-') as directory:
        root = Path(directory)
        (root / 'bindings').mkdir()
        shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
        library = args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib'
        (root / 'bindings' / library.name).symlink_to(library)
        helper = root / 'credential'
        marker = root / 'forbidden-secret-read'
        helper.write_text(f'#!{sys.executable}\nfrom pathlib import Path\nPath({str(marker)!r}).touch()\nraise SystemExit(1)\n')
        helper.chmod(0o700)
        os.environ.clear()
        os.environ.update(HOME=str(root), PATH='/usr/bin:/bin')
        spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
        bindings = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = bindings
        spec.loader.exec_module(bindings)
        application = None
        try:
            home = root / 'home'
            home.mkdir()
            endpoint = 'https://synthetic.invalid/v1'
            config_path = home / 'generic-config.json'
            legacy = {'schemaVersion': 3, 'obsolete': 'must disappear', 'gateways': [{'id': 'old'}]}
            config_path.write_text(json.dumps(legacy))
            source_file = root / 'original-harness.json'
            source_file.write_text('synthetic source stays unchanged')
            options = bindings.BindingOptions(home_directory=str(home),
                resources_directory=str(args.bundle / 'Contents/Resources'), credential_resolver=str(helper))
            application = bindings.VeluneApplication.open(options)
            reset = json.loads(config_path.read_text())
            assert reset == {'schemaVersion': 4, 'gateways': [], 'runtimeInstances': [], 'authenticationBindings': []}
            assert sorted(item.name for item in home.iterdir() if item.is_file()) == ['generic-config.json', 'runtime.lock']
            assert source_file.read_text() == 'synthetic source stays unchanged'
            protocol = bindings.BindingGatewayProtocol.CHAT_COMPLETIONS_V1
            application.configure_api_key_binding('external', 'External', 'external-key',
                protocol, endpoint, False, None)
            application.upsert_gateway(bindings.BindingGatewayConfig(id='synthetic', name='Synthetic',
                models=[], routes=[], providers=[bindings.BindingProviderDefinition(id='a', name='A',
                    protocol=protocol, endpoint=endpoint, authentication_id='external', models=[])],
                failover=bindings.BindingFailoverPolicy(mode=bindings.BindingFailoverMode.DISABLED)))

            def rejected(operation):
                before = config_path.read_bytes()
                try:
                    operation()
                except bindings.BindingError:
                    assert config_path.read_bytes() == before, 'rejected mutation changed persisted state'
                    return
                raise AssertionError('invalid operation succeeded')

            gateway, = application.list().gateways
            actual_id = gateway.providers[0].authentication_id
            gateway.providers[0].authentication_id = 'unregistered-keychain-reference'
            rejected(lambda: application.upsert_gateway(gateway))
            gateway.providers[0].authentication_id = actual_id
            gateway.providers[0].endpoint = 'https://different.invalid/v1'
            rejected(lambda: application.upsert_gateway(gateway))
            rejected(lambda: application.delete_authentication_binding(actual_id))
            gateway.providers[0].endpoint = endpoint
            protocol = bindings.BindingGatewayProtocol.CHAT_COMPLETIONS_V1
            old_ref = 'credential-11111111-1111-4111-8111-111111111111'
            new_ref = 'credential-22222222-2222-4222-8222-222222222222'
            created = application.configure_api_key_binding('managed', 'Managed', old_ref,
                protocol, endpoint, True, None)
            assert not created.obsolete_owned_keychain_refs
            replaced = application.configure_api_key_binding('managed', 'Managed replacement', new_ref,
                protocol, endpoint, True, created.binding.generation)
            assert replaced.obsolete_owned_keychain_refs == [old_ref]
            assert replaced.binding.generation > created.binding.generation
            rejected(lambda: application.configure_api_key_binding('managed', 'Stale', old_ref,
                protocol, endpoint, True, created.binding.generation))
            renamed = application.rename_authentication_binding('managed', 'Renamed')
            assert renamed.binding.name == 'Renamed'
            deleted = application.delete_authentication_binding('managed')
            assert deleted.obsolete_owned_keychain_refs == [new_ref]
            shared_a = application.configure_api_key_binding('shared-a', 'Shared A', old_ref,
                protocol, endpoint, True, None)
            application.configure_api_key_binding('shared-b', 'Shared B', old_ref,
                protocol, endpoint, True, None)
            shared_replaced = application.configure_api_key_binding('shared-a', 'Shared A', new_ref,
                protocol, endpoint, True, shared_a.binding.generation)
            assert not shared_replaced.obsolete_owned_keychain_refs, 'another resource still uses the old item'
            shared_deleted = application.delete_authentication_binding('shared-b')
            assert shared_deleted.obsolete_owned_keychain_refs == [old_ref]
            application.delete_authentication_binding('shared-a')
            external = next(item for item in application.list().authentication_bindings if item.id == 'external')
            legacy_replaced = application.configure_api_key_binding('external', 'External replacement', new_ref,
                protocol, endpoint, True, external.generation)
            assert not legacy_replaced.obsolete_owned_keychain_refs, 'external legacy key scheduled for deletion'
            gateway, = application.list().gateways
            gateway.models = [bindings.BindingModelDefinition(record_key='', nickname='Shared real model',
                icon=None, context_window=None, max_output_tokens=None)]
            application.upsert_gateway(gateway)
            gateway, = application.list().gateways
            record_key = gateway.models[0].record_key
            assert record_key and record_key != 'Shared real model'
            gateway.providers[0].models = [bindings.BindingProviderModelBinding(
                model_record_key=record_key, provider_model_id='org/model:版本-1',
                context_window=None, max_output_tokens=None, reasoning=None, adapter_metadata_json=None)]
            gateway.providers.append(bindings.BindingProviderDefinition(id='b', name='B',
                protocol=protocol, endpoint=endpoint, authentication_id='external',
                models=[bindings.BindingProviderModelBinding(model_record_key=record_key,
                    provider_model_id='different-api-name', context_window=32768, max_output_tokens=4096,
                    reasoning=None, adapter_metadata_json=None)]))
            gateway.routes = [bindings.BindingRoute(model_record_key=record_key, provider_id='a')]
            application.upsert_gateway(gateway)
            gateway, = application.list().gateways
            assert gateway.models[0].context_window is None
            assert gateway.providers[0].models[0].provider_model_id == 'org/model:版本-1'
            assert gateway.providers[0].models[0].context_window is None
            assert gateway.providers[1].models[0].context_window == 32768
            application.shutdown()
            application = bindings.VeluneApplication.open(options)
            assert len(application.list().authentication_bindings) == 1
            application.shutdown()
            application = None
            for label, contents in [('future', json.dumps({'schemaVersion': 5})),
                                    ('malformed', '{broken'),
                                    ('old-fields', json.dumps({**reset, 'obsolete': True}))]:
                invalid_home = root / label
                invalid_home.mkdir()
                invalid_path = invalid_home / 'generic-config.json'
                invalid_path.write_text(contents)
                before = invalid_path.read_bytes()
                try:
                    application = bindings.VeluneApplication.open(bindings.BindingOptions(
                        home_directory=str(invalid_home), resources_directory=str(args.bundle / 'Contents/Resources'),
                        credential_resolver=str(helper)))
                except bindings.BindingError:
                    assert invalid_path.read_bytes() == before
                else:
                    raise AssertionError(label + ' configuration accepted')
            assert not marker.exists()
            print(json.dumps({'acceptance': 'PASSED', 'schemaFourHardCutoffReset': True,
                'noOldFileOrBackup': True, 'originalHarnessPreserved': True,
                'providersContainOnlyRegisteredId': True, 'unknownResourceAndTargetRejected': True,
                'usedResourceDeletionRejected': True, 'replacementCleanupAndGeneration': True,
                'staleReplacementRejected': True, 'sharedOwnedItemNotDeletedWhileReferenced': True,
                'externalSecretNotDeleted': True, 'persistenceReopened': True, 'generatedHiddenRecordKey': True,
                'crossProviderIdsAndCapabilitiesIndependent': True,
                'futureMalformedAndOldFieldsRejectedWithoutMutation': True, 'secretAndSourceReads': 0}))
        finally:
            if application:
                application.shutdown()
            os.environ.clear()
            os.environ.update(environment)


if __name__ == '__main__':
    main()
