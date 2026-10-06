#!/usr/bin/env python3
"""Explicit synthetic discovery/config acceptance; no user config or credentials."""
import argparse, importlib.util, json, os, sys, tempfile
from pathlib import Path

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bindings', type=Path, required=True)
    parser.add_argument('--library', type=Path, required=True)
    parser.add_argument('--resources', type=Path, required=True)
    parser.add_argument('--node', type=Path, required=True)
    parser.add_argument('--pi', type=Path, required=True, help='external Pi CLI from the selected installation')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='velune-discovery-') as directory:
        root = Path(directory); home = root/'home'; home.mkdir()
        module = root/'velune_bindings.py'; module.write_bytes((args.bindings/'velune_bindings.py').read_bytes())
        (root/'libvelune_bindings.dylib').symlink_to(args.library.resolve())
        os.environ['HOME'] = str(home)
        spec = importlib.util.spec_from_file_location('velune_bindings', module)
        b = importlib.util.module_from_spec(spec); sys.modules[spec.name] = b; spec.loader.exec_module(b)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(home), resources_directory=str(args.resources.resolve())))
        assert app.list().conversation_browser_group_limit == 20
        assert any(p.id == b.BindingGatewayProtocol.MESSAGES_V1 and p.supported for p in app.list().protocols)
        assert app.set_conversation_browser_group_limit(7) == 7
        try: app.set_conversation_browser_group_limit(0)
        except b.BindingError: pass
        else: raise AssertionError('zero limit accepted')
        hints = app.runtime_discovery_hints(str(home), {})
        assert {h.family_id:h.agent_directory for h in hints} == {'pi':str(home/'.pi/agent'),'codex':str(home/'.codex'),'deepseek-harness':str(home/'.dsh')}
        assert not any(h.directory_exists for h in hints)
        agent = home/'.pi/agent'; agent.mkdir(parents=True)
        # An executable that supports only --version cannot accidentally start a session.
        def cli(name, version):
            package = root / name
            path = package / 'dist/bundle/cli.js'
            path.parent.mkdir(parents=True)
            (package / 'package.json').write_text(json.dumps({
                'name': '@earendil-works/pi-coding-agent', 'version': version,
                'bin': {'pi': 'dist/bundle/cli.js'},
                'exports': {'.': {'import': './dist/index.js'}},
            }))
            path.write_text("if(process.argv[2] !== '--version') process.exit(91); console.log('"+version+"');\n")
            path.chmod(0o700)
            return str(path)
        valid = cli('supported.mjs','1.0.2'); invalid = cli('unsupported.mjs','2.0.0')
        probe = lambda path: b.BindingRuntimeDiscoveryProbe(family_id='pi',binary=path,node_binary=str(args.node.resolve()),agent_directory=str(agent))
        candidates = app.discover_runtimes([probe(valid),probe(invalid)])
        assert candidates[0].supported and candidates[0].version == '1.0.2' and not candidates[0].already_configured
        assert not candidates[1].supported and candidates[1].version == '2.0.0'
        external = app.discover_runtimes([probe(str(args.pi))])[0]
        assert external.supported and external.version == '1.0.2', (external.supported, external.version)
        assert external.runtime.settings['binary'] == str(args.pi)
        assert not app.list().runtime_instances, 'detection saved configuration'
        runtime = candidates[0].runtime
        app.upsert_runtime(runtime)
        assert app.discover_runtimes([probe(valid)])[0].already_configured
        assert app.runtime_discovery_hints(str(home),{})[0].directory_exists
        assert not (agent/'auth.json').exists()
        app.shutdown()
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(home), resources_directory=str(args.resources.resolve())))
        assert app.list().conversation_browser_group_limit == 7
        assert len(app.list().runtime_instances) == 1
        app.shutdown()
        print('synthetic discovery, version rejection, explicit import, dedup and persisted group limit passed')
if __name__ == '__main__': main()
