#!/usr/bin/env python3
"""Explicit public GET and isolated template-copy acceptance; never a CI entry point.

Uses synthetic parser data and temporary application HOME only. --public-get
opts into a credential-free GET to the fixed https://models.dev/api.json URL.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

PROBE = r'''
use velune_application::model_catalog::parse_models_dev;
fn main() {
    let bytes = br#"{
      "map-key-a": {"id":"source-a","name":"Source A","models": {
        "not-the-id": {"id":"shared/api-id","name":"Model","reasoning":true,"limit":{"context":0,"output":null},"reasoning_options":[{"type":"toggle"}]}
      }},
      "map-key-b": {"id":"source-b","name":"Source B","models": {
        "another-key": {"id":"shared/api-id","name":"Model","limit":{"context":1000,"output":200},"reasoning_options":[{"type":"effort","values":[null,"low","high"]}]}
      }}
    }"#;
    let models = parse_models_dev(bytes).unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].model_id, "shared/api-id");
    assert_eq!(models[1].model_id, "shared/api-id");
    assert_ne!(models[0].source_provider_id, models[1].source_provider_id);
    assert_eq!(models[0].context_window, None);
    assert_eq!(models[0].max_output_tokens, None);
    assert_eq!(models[0].reasoning_levels, None);
    assert_eq!(models[1].reasoning_levels.as_ref().unwrap(), &["low", "high"]);
    assert!(parse_models_dev(b"not-json").is_err());
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bindings', type=Path, required=True)
    binaries = parser.add_mutually_exclusive_group(required=True)
    binaries.add_argument('--library', type=Path)
    binaries.add_argument('--bundle', type=Path)
    parser.add_argument('--public-get', action='store_true')
    args = parser.parse_args()
    if args.bundle:
        args.library = args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib'
    repo = Path(__file__).resolve().parent.parent
    artifact_lines = subprocess.check_output(['cargo', 'build', '--locked', '-p',
        'velune-application', '--message-format=json'], cwd=repo, text=True)
    artifacts = [json.loads(line) for line in artifact_lines.splitlines() if line.startswith('{')]
    library = next(Path(file) for item in artifacts if item.get('reason') == 'compiler-artifact'
        and item['target']['name'] == 'velune_application' for file in item['filenames'] if file.endswith('.rlib'))
    with tempfile.TemporaryDirectory(prefix='velune-catalog-manual-') as directory:
        root = Path(directory)
        probe = root / 'probe.rs'
        probe.write_text(PROBE)
        subprocess.run(['rustc', '--edition=2024', str(probe), '--extern',
            f'velune_application={library}', '-L', f'dependency={repo / "target/debug/deps"}',
            '-o', str(root / 'probe')], check=True)
        subprocess.run([str(root / 'probe')], check=True)
        if not args.public_get:
            print(json.dumps({'syntheticParserBoundaries': True, 'publicGet': False}))
            return
        shutil.copy(args.bindings / 'velune_bindings.py', root)
        (root / args.library.name).symlink_to(args.library.resolve())
        spec = importlib.util.spec_from_file_location('velune_bindings', root / 'velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        os.environ['HOME'] = str(root)
        os.environ.pop('VELUNE_HOME', None)
        resources = root / 'resources'
        resources.mkdir()
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'home'), resources_directory=str(resources)))
        try:
            config = root / 'home/generic-config.json'
            before = config.read_bytes() if config.exists() else None
            entries = app.fetch_public_model_catalog()
            assert entries and len({(m.source_provider_id, m.model_id) for m in entries}) == len(entries)
            assert (config.read_bytes() if config.exists() else None) == before
            candidate = next(m for m in entries if m.source_provider_id == 'deepseek')
            saved = app.save_model_template(b.BindingModelTemplate(id='', name=f'{candidate.name} · {candidate.source_provider_name} (models.dev)',
                suggested_provider_model_id=candidate.model_id, nickname=candidate.name,
                icon=None, context_window=candidate.context_window,
                max_output_tokens=candidate.max_output_tokens, reasoning_levels=candidate.reasoning_levels))
            assert len(saved) == 1 and saved[0].suggested_provider_model_id == candidate.model_id
            snapshot = app.list()
            assert not any(g.providers for g in snapshot.gateways)
            assert snapshot.model_templates[0].reasoning_levels == candidate.reasoning_levels
            saved[0].nickname = 'Synthetic edited template'
            app.save_model_template(saved[0])
            app.shutdown()
            try:
                app.fetch_public_model_catalog()
            except b.BindingError.Diagnostic as error:
                assert error.kind == b.BindingFailureKind.CLOSED
                assert error.code == "application_closed" and error.operation_id
            else:
                raise AssertionError('closed application allowed catalog retrieval')
            app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'home'), resources_directory=str(resources)))
            reopened = app.list()
            assert reopened.model_templates[0].nickname == 'Synthetic edited template'
            assert reopened.model_templates[0].suggested_provider_model_id == candidate.model_id
            assert not any(g.providers for g in reopened.gateways)
            print(json.dumps({'syntheticParserBoundaries': True, 'publicGet': True,
                'providerScopedModels': len(entries), 'retrievalDoesNotChangeConfiguration': True,
                'editableTemplateCopy': True, 'reopenedTemplateSnapshot': True, 'closedCatalogRetrievalRejected': True, 'configuredProviderCount': 0}))
        finally:
            app.shutdown()


if __name__ == '__main__':
    main()
