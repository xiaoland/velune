#!/usr/bin/env python3
"""Manual BindingApp error-cause probe using an isolated failing helper."""
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
    parser.add_argument("--bindings", required=True, type=Path)
    parser.add_argument("--library", required=True, type=Path)
    parser.add_argument("--node", required=True, type=Path)
    parser.add_argument("--pi", required=True, type=Path)
    args = parser.parse_args()
    unique = "SYNTHETIC_PROVIDER_CAUSE_7f3a"
    with tempfile.TemporaryDirectory(prefix="velune-error-diagnostics-") as directory:
        root = Path(directory)
        home, resources, agent = root / "home", root / "resources", root / "agent"
        home.mkdir(); resources.mkdir(); agent.mkdir()
        (resources / "pi_provider_import.mjs").write_text(
            "process.stderr.write(JSON.stringify({contractVersion:1,diagnostic:{code:'adapter_failed',phase:'models'},detail:'%s'})); process.exitCode=1;\n" % unique
        )
        binding_file = root / "velune_bindings.py"
        shutil.copy(args.bindings / "velune_bindings.py", binding_file)
        shutil.copy(args.library, root / "libvelune_bindings.dylib")
        old_home = os.environ.get("HOME")
        os.environ["HOME"] = str(home)
        try:
            spec = importlib.util.spec_from_file_location("velune_bindings", binding_file)
            module = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = module
            spec.loader.exec_module(module)
            app = module.VeluneApplication.open(module.BindingOptions(home_directory=str(home), resources_directory=str(resources)))
            app.upsert_runtime(module.BindingRuntimeInstance(
                enabled=True, id="pi", name="Synthetic Pi", type_id="pi-1.0.2",
                gateway_id="default", settings={"agentDir": str(agent), "nodeBinary": str(args.node), "binary": str(args.pi)}
            ))
            source = module.BindingProviderImportSource(kind="harness", harness_type_id="pi", source_instance_id="pi", provider_id=None, settings={})
            try:
                app.preview_provider_import("default", source)
            except Exception as error:
                text = str(error)
                assert unique in text, text
                print("PASS", text)
            else:
                raise AssertionError("synthetic provider import unexpectedly succeeded")
            # Exercise the native Pi history branch, including the application
            # context mapping that used to erase the helper's original failure.
            synthetic_session = agent / "synthetic.jsonl"
            synthetic_session.write_text('{}\n')
            mode = resources / "mode"
            mode.write_text("read-error")
            helper = """import {readFileSync} from 'node:fs';
const mode=readFileSync(new URL('./mode',import.meta.url),'utf8');
const inspect=process.argv.includes('--inspect-session');
if(mode==='list-error'||(inspect&&mode==='read-error')){process.stderr.write(inspect?'SYNTHETIC_HISTORY_READ_CAUSE':'SYNTHETIC_HISTORY_LIST_CAUSE');process.exitCode=1;}
else if(inspect&&mode==='invalid-json'){process.stdout.write('{ invalid json');}
else if(inspect&&mode==='valid-history'){process.stdout.write(JSON.stringify({name:'Synthetic',cwd:CWD,messages:[{role:'user',content:'Question',timestamp:1},{role:'assistant',content:[{type:'thinking',thinking:'Work'}],timestamp:2},{role:'assistant',content:'Answer',timestamp:3},{role:'system',content:'Handoff status',timestamp:4}]}));}
else{process.stdout.write(JSON.stringify({sessions:[{path:SESSION,cwd:CWD,name:'Synthetic',firstMessage:'Synthetic',modifiedUnixMs:1,createdUnixMs:1}]}));}
""".replace('SESSION', json.dumps(str(synthetic_session))).replace('CWD', json.dumps(str(agent)))
            (resources / "pi_sessions.mjs").write_text(helper)
            for operation, expected in (("read-error", "SYNTHETIC_HISTORY_READ_CAUSE"), ("invalid-json", "line 1 column")):
                mode.write_text(operation)
                try:
                    app.open_conversation("pi", "pi:" + str(synthetic_session))
                except Exception as error:
                    assert expected in str(error), str(error)
                else:
                    raise AssertionError("synthetic history read unexpectedly succeeded")
            mode.write_text("list-error")
            listing = app.list()
            assert len(listing.history_failures) == 1
            assert "SYNTHETIC_HISTORY_LIST_CAUSE" in listing.history_failures[0].detail
            print("PASS native Pi history read/list/parser causes")
            mode.write_text("valid-history")
            snapshot = app.open_conversation("pi", "pi:" + str(synthetic_session)).snapshot
            turn = snapshot.transcript_turns[0]
            assert turn.user_message_id == snapshot.messages[0].id
            assert turn.work_message_ids == [snapshot.messages[1].id]
            assert turn.last_message_id == snapshot.messages[2].id
            assert snapshot.messages[3].id not in turn.work_message_ids
            assert turn.duration_ms is None and not turn.is_running
            print("PASS typed history turn keeps answer and trailing system status visible")
            app.shutdown()
        finally:
            if old_home is None:
                os.environ.pop("HOME", None)
            else:
                os.environ["HOME"] = old_home


if __name__ == "__main__":
    main()
