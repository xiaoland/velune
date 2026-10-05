#!/usr/bin/env python3
"""Explicit synthetic UniFFI acceptance of the central authentication repository.

All files live in a temporary home. The credential helper fails if invoked:
migration and configuration operations must not read any secret or source file.
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
            source = {'kind': 'harness', 'harnessTypeId': 'pi', 'providerId': 'synthetic',
                      'settings': {'authPath': str(root / 'nonexistent-source/auth.json'),
                                   'credentialKind': 'literal_api_key'}}
            def provider(identity, credential_source=None, reference=None):
                return {'id': identity, 'name': identity, 'protocol': 'chatCompletionsV1',
                        'endpoint': endpoint, 'models': [], 'credentialSource': credential_source,
                        'credentialRef': reference, 'credentialGeneration': 4}
            legacy = {'schemaVersion': 2, 'runtimeInstances': [], 'gateways': [
                {'id': 'synthetic', 'name': 'Synthetic', 'models': [], 'routes': [],
                 'failover': {'mode': 'disabled'}, 'providers': [
                     provider('a', source), provider('b', source), provider('c', reference='external-key')]}]}
            config_path = home / 'generic-config.json'
            config_path.write_text(json.dumps(legacy))
            options = bindings.BindingOptions(home_directory=str(home),
                resources_directory=str(args.bundle / 'Contents/Resources'), credential_resolver=str(helper))
            application = bindings.VeluneApplication.open(options)
            migrated = json.loads(config_path.read_text())
            assert migrated['schemaVersion'] == 3
            assert len(migrated['authenticationBindings']) == 3
            ids = [item['authenticationId'] for item in migrated['gateways'][0]['providers']]
            assert len(set(ids)) == 3, 'same source path merged independent identities'
            assert all(not any(key.startswith('credential') for key in item)
                       for item in migrated['gateways'][0]['providers'])
            assert not marker.exists() and not (root / 'nonexistent-source').exists()

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
            legacy_replaced = application.configure_api_key_binding(ids[2], 'Legacy replacement', new_ref,
                protocol, endpoint, True, 4)
            assert not legacy_replaced.obsolete_owned_keychain_refs, 'external legacy key scheduled for deletion'
            application.shutdown()
            application = bindings.VeluneApplication.open(options)
            assert len(application.list().authentication_bindings) == 3
            application.shutdown()
            application = None
            invalid_home = root / 'invalid'
            invalid_home.mkdir()
            invalid = json.loads(json.dumps(legacy))
            invalid['gateways'][0]['providers'][0]['credentialRef'] = 'conflicting-source'
            invalid_path = invalid_home / 'generic-config.json'
            invalid_path.write_text(json.dumps(invalid))
            before = invalid_path.read_bytes()
            try:
                application = bindings.VeluneApplication.open(bindings.BindingOptions(
                    home_directory=str(invalid_home), resources_directory=str(args.bundle / 'Contents/Resources'),
                    credential_resolver=str(helper)))
            except bindings.BindingError:
                assert invalid_path.read_bytes() == before
            else:
                raise AssertionError('conflicting legacy authentication migrated')
            assert not marker.exists()
            print(json.dumps({'acceptance': 'PASSED', 'schemaTwoMigration': True,
                'independentIdentitiesPreserved': True, 'providersContainOnlyRegisteredId': True,
                'unknownResourceAndTargetRejected': True, 'usedResourceDeletionRejected': True,
                'replacementCleanupAndGeneration': True, 'staleReplacementRejected': True,
                'sharedOwnedItemNotDeletedWhileReferenced': True,
                'externalLegacySecretNotDeleted': True, 'persistenceReopened': True,
                'invalidMigrationLeavesFileUnchanged': True, 'secretAndSourceReads': 0}))
        finally:
            if application:
                application.shutdown()
            os.environ.clear()
            os.environ.update(environment)


if __name__ == '__main__':
    main()
