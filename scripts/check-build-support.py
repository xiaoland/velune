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
            args=argparse.Namespace(platform='macos',profile='dev',command=[sys.executable,'-c','pass'])
            self.assertEqual(support.run(args),0)
            success=(support.STATE/'results/macos-dev-success.json').read_bytes()
            args.command=[sys.executable,'-c',f'from pathlib import Path; Path({str(source)!r}).write_text("after")']
            self.assertEqual(support.run(args),3)
            self.assertEqual((support.STATE/'results/macos-dev-success.json').read_bytes(),success)
            self.assertFalse(json.loads((support.STATE/'results/macos-dev.json').read_text())['current'])
            args.command=[sys.executable,'-c',f'from pathlib import Path; p=Path({str(source)!r}); old=p.read_bytes(); p.write_bytes(b"temporary edit"); p.write_bytes(old)']
            self.assertEqual(support.run(args),3, 'Restored bytes must not hide a source edit during compilation')
        finally:
            support.ROOT, support.STATE = old_root, old_state


if __name__=='__main__': unittest.main()
