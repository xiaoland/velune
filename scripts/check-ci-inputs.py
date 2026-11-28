#!/usr/bin/env python3
"""Check CI routing against independent examples of build and test inputs."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('ci-inputs.py').resolve()
spec = importlib.util.spec_from_file_location('ci_inputs', SCRIPT)
inputs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inputs)


class RustInputChecks(unittest.TestCase):
    def test_compiled_sources_helpers_fixtures_and_ffi_consumers_run(self):
        # The non-Rust examples are consumed by include_str!/include_bytes!,
        # build scripts, generated-language contracts or the acceptance runner.
        for name in (
            'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'clippy.toml', '.cargo/config.toml',
            '.github/workflows/pr-checks.yml', 'scripts/ci-inputs.py', 'scripts/build-support.py',
            'scripts/run-migrations.py', 'scripts/migrations/composer-state.py',
            'packages/agent-runtime/resources/codex_context.py',
            'packages/application-api/src/command/traits_expected.tsv',
            'packages/gateway/tests/fixtures/gemini/google-text.json',
            'tools/bindgen/Cargo.toml', 'third-party/iroh/SOURCE.json',
            'tests/acceptance/cases/history_cache_pressure.py',
        ):
            with self.subTest(path=name):
                self.assertTrue(inputs.requires_rust([name]), name)

    def test_unrelated_docs_and_native_only_changes_do_not_compile_rust(self):
        for names in (['docs/prd/conversations.md'], ['app/mac/Composer.swift'],
                      ['scripts/check-ci-inputs.py', 'scripts/check-macos-dev.py']):
            with self.subTest(paths=names):
                self.assertFalse(inputs.requires_rust(names))

    def test_cli_keeps_deleted_source_of_a_rename_and_appends_output(self):
        with tempfile.TemporaryDirectory(prefix='velune-ci-inputs-') as temporary:
            repo = Path(temporary)
            def git(*args):
                return subprocess.check_output(['git', *args], cwd=repo, text=True).strip()
            git('init', '-q')
            git('config', 'user.email', 'synthetic@example.invalid')
            git('config', 'user.name', 'Synthetic CI check')
            source = repo / 'packages/ai/src/model.rs'
            source.parent.mkdir(parents=True)
            source.write_text('// unchanged content moved out of the Rust package\n')
            git('add', '.')
            git('commit', '-qm', 'base')
            base = git('rev-parse', 'HEAD')
            destination = repo / 'docs/model.txt'
            destination.parent.mkdir()
            git('mv', str(source), str(destination))
            git('commit', '-qm', 'move source outside the package')
            output = repo / 'github-output'
            output.write_text('previous=value\n')
            subprocess.run([sys.executable, str(SCRIPT), base, str(output)], cwd=repo, check=True)
            self.assertEqual(output.read_text(), 'previous=value\nrust=true\n')


if __name__ == '__main__':
    unittest.main()
