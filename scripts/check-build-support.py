#!/usr/bin/env python3
"""Exercise build output and provenance promises with small isolated inputs."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('support', Path(__file__).with_name('build-support.py'))
support = importlib.util.module_from_spec(spec)
spec.loader.exec_module(support)


class BuildSupportChecks(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='velune-build-support-')
        self.root = Path(self.temporary.name)
    def tearDown(self):
        self.temporary.cleanup()
    def test_stable_generated_inputs_and_selective_prune(self):
        source, destination = self.root/'source', self.root/'destination'
        source.mkdir(); destination.mkdir()
        (source/'New.swift').write_text('new')
        (source/'Bridge.h').write_text('header')
        (destination/'Old.swift').write_text('old')
        (destination/'module.modulemap').write_text('owned by platform')
        support.sync(source, destination, True, ['*.swift'])
        stamp = (destination/'New.swift').stat().st_mtime_ns
        support.sync(source, destination, True, ['*.swift'])
        self.assertEqual((destination/'New.swift').stat().st_mtime_ns, stamp)
        self.assertFalse((destination/'Old.swift').exists())
        self.assertFalse((destination/'Bridge.h').exists())
        self.assertEqual((destination/'module.modulemap').read_text(), 'owned by platform')
    def test_paired_outputs_survive_failure_and_reject_stale_library(self):
        # The fake executable controls a generator boundary, not a fake build
        # cache: actual generation, publication, verification and failure run.
        generator = self.root/'generator'
        generator.write_text('#!'+sys.executable+'\nimport pathlib,sys\na=sys.argv\np=pathlib.Path(a[a.index("--library")+1])\nif p.read_text()=="fail": raise SystemExit(7)\nout=pathlib.Path(a[a.index("--out-dir")+1]);out.mkdir(exist_ok=True)\n(out/"Probe.swift").write_text("same interface")\n')
        generator.chmod(0o755)
        library = self.root/'library'; library.write_text('first')
        directory = self.root/'generated'
        args = argparse.Namespace(library=str(library), language='cs', generator=str(generator), out_dir=str(directory), artifact=None)
        support.bindings(args)
        stamp = (directory/'Probe.swift').stat().st_mtime_ns
        library.write_text('internal implementation change')
        support.bindings(args)
        self.assertEqual((directory/'Probe.swift').stat().st_mtime_ns, stamp)
        support.bindings(args, verify=True)
        library.write_text('fail')
        with self.assertRaises(subprocess.CalledProcessError): support.bindings(args)
        self.assertEqual((directory/'Probe.swift').read_text(), 'same interface')
        with self.assertRaises(ValueError): support.bindings(args, verify=True)
    def test_packaged_artifact_is_part_of_pair(self):
        directory=self.root/'generated'; directory.mkdir(); (directory/'Probe.swift').write_text('interface')
        artifact=self.root/'packaged.so'; artifact.write_text('paired')
        stamp={'files':{'Probe.swift':support.digest(directory/'Probe.swift')},'artifacts':{str(artifact):support.digest(artifact)}}
        self.assertTrue(support.generated_valid(directory,stamp))
        artifact.write_text('wrong native copy')
        self.assertFalse(support.generated_valid(directory,stamp))
    def test_changed_input_cannot_publish_current_success(self):
        old_root, old_state = support.ROOT, support.STATE
        support.ROOT, support.STATE = self.root, self.root/'target/build-state'
        try:
            subprocess.run(['git','init','-q',str(self.root)],check=True)
            source=self.root/'app/mac/Probe.swift'; source.parent.mkdir(parents=True); source.write_text('before')
            args=argparse.Namespace(platform='macos',profile='dev',purpose='development',android_native_only=False,command=[sys.executable,'-c','pass'])
            self.assertEqual(support.run(args),0)
            result = support.STATE/'results/macos-dev-development.json'
            successful_result = support.STATE/'results/macos-dev-development-success.json'
            success=successful_result.read_bytes()
            args.command=[sys.executable,'-c',f'from pathlib import Path; Path({str(source)!r}).write_text("after")']
            self.assertEqual(support.run(args),3)
            self.assertEqual(successful_result.read_bytes(),success)
            self.assertFalse(json.loads(result.read_text())['current'])
            args.command=[sys.executable,'-c',f'from pathlib import Path; p=Path({str(source)!r}); old=p.read_bytes(); p.write_bytes(b"temporary edit"); p.write_bytes(old)']
            self.assertEqual(support.run(args),3, 'Restored bytes must not hide a source edit during compilation')
        finally:
            support.ROOT, support.STATE = old_root, old_state

    def test_android_build_receipts_follow_the_outputs_of_each_entry_point(self):
        state = self.root / 'target/build-state'
        results = state / 'results'
        library = self.root / 'target/android-native/arm64-v8a/libvelune_bindings.so'
        bindings = self.root / 'target/android-bindings/.velune-bindings.json'
        native_result = results / 'android-native-dev.json'
        native_success = results / 'android-native-dev-success.json'
        apk_result = results / 'android-dev.json'
        apk_success = results / 'android-dev-success.json'
        args = argparse.Namespace(platform='android', profile='dev', purpose='development',
                                  android_native_only=True, command=[sys.executable, '-c', 'pass'])
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        (self.root / 'LICENSE').write_text('original license')
        for path in (library, bindings):
            path.parent.mkdir(parents=True)
            path.write_text('synthetic artifact')

        with patch.multiple(support, ROOT=self.root, STATE=state):
            self.assertEqual(support.run(args), 0)
            native = json.loads(native_result.read_text())
            self.assertEqual(set(native['artifacts']), {
                library.relative_to(self.root).as_posix(),
                bindings.relative_to(self.root).as_posix(),
            })
            self.assertFalse(apk_result.exists())
            bindings.unlink()
            self.assertEqual(support.run(args), 1, 'a missing binding manifest must fail a native build')
            self.assertEqual(json.loads(native_result.read_text())['exit_code'], 1)
            self.assertEqual(json.loads(native_success.read_text()), native)
            bindings.write_text('synthetic artifact')

            args.android_native_only = False
            args.command = [sys.executable, '-c', 'raise SystemExit(17)']
            self.assertEqual(support.run(args), 17)
            failure = json.loads(apk_result.read_text())
            self.assertEqual(failure['exit_code'], 17)
            self.assertEqual(failure['artifacts'], {})
            self.assertFalse(apk_success.exists())

            args.command = [sys.executable, '-c', 'pass']
            self.assertEqual(support.run(args), 1, 'a missing APK must still fail a full build')
            self.assertIn('output-metadata.json', json.loads(apk_result.read_text())['artifact_error'])
            self.assertFalse(apk_success.exists())

            apk_dir = self.root / 'app/android/app/build/outputs/apk/debug'
            apk_dir.mkdir(parents=True)
            (apk_dir / 'app-debug.apk').write_text('synthetic APK')
            (apk_dir / 'output-metadata.json').write_text(json.dumps({
                'variantName': 'debug', 'elements': [{'outputFile': 'app-debug.apk'}],
            }))
            self.assertEqual(support.run(args), 0)
            success = apk_success.read_bytes()
            self.assertIn('app/android/app/build/outputs/apk/debug/app-debug.apk',
                          json.loads(success)['artifacts'])
            args.command = [sys.executable, '-c', 'raise SystemExit(17)']
            self.assertEqual(support.run(args), 17)
            latest_apk_result = apk_result.read_bytes()
            self.assertEqual(apk_success.read_bytes(), success)

            args.android_native_only = True
            args.command = [sys.executable, '-c', 'pass']
            self.assertEqual(support.run(args), 0)
            self.assertEqual(apk_result.read_bytes(), latest_apk_result)
            self.assertEqual(apk_success.read_bytes(), success)
            self.assertEqual(json.loads(native_result.read_text())['artifacts'], native['artifacts'])

            args.command = [sys.executable, '-c', 'from pathlib import Path; Path("LICENSE").write_text("updated")']
            self.assertEqual(support.run(args), 3)
            self.assertEqual(json.loads(native_result.read_text())['changed_inputs'], ['LICENSE'])


if __name__=='__main__': unittest.main()
