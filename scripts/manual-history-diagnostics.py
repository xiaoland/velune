#!/usr/bin/env python3
"""Manual, synthetic history bridge acceptance; never a CI test."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def invoke(node: Path, helper: Path, request: dict) -> dict:
    result = subprocess.run(
        [str(node), str(helper)], input=json.dumps(request), text=True,
        capture_output=True, check=True,
    )
    assert result.stderr == "", result.stderr
    return json.loads(result.stdout)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--node", type=Path, required=True)
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--bindings", type=Path)
    parser.add_argument("--library", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="velune-history-diagnostic-") as directory:
        root = Path(directory)
        home = root / "home"
        sessions = root / "sessions"
        home.mkdir()
        sessions.mkdir()
        good = sessions / "good.jsonl"
        good.write_text(
            '{"timestamp":"2026-10-06T00:00:00.000Z","type":"session_meta",'
            '"payload":{"id":"synthetic-good","cwd":"/tmp/synthetic"}}\n'
            '{"timestamp":"2026-10-06T00:00:01.000Z","type":"event_msg",'
            '"payload":{"type":"user_message","message":"hello"}}\n',
            encoding="utf-8",
        )
        # huihua keeps malformed JSONL as a diagnostic, so this is still a
        # readable source and proves the projection does not expose raw errors.
        (sessions / "corrupt.jsonl").write_bytes(
            b'{"timestamp":"2026-10-06T00:00:00.000Z","type":"session_meta",'
            b'"payload":{"id":"synthetic-corrupt","cwd":"/tmp/synthetic"}}\n'
            b'{not-json}\n'
        )
        request = {"operation": "list", "provider": "codex", "home": str(home), "roots": [str(sessions)]}
        listed = invoke(args.node, args.helper, request)
        assert listed["contractVersion"] == 1
        assert {item["nativeId"] for item in listed["sessions"]} == {"synthetic-good", "synthetic-corrupt"}
        assert listed["failures"] == []
        stale = dict(request, operation="read", nativeId="synthetic-missing")
        assert invoke(args.node, args.helper, stale) == {
            "contractVersion": 1, "error": {"code": "session_not_found"}
        }
        if args.bindings and args.library:
            # A tiny explicit shim exercises application -> bindings diagnostic
            # mapping without reading a real provider or session.
            shim = root / "huihua_sessions.mjs"
            shim.write_text(
                "let s=''; for await (const c of process.stdin) s+=c; const r=JSON.parse(s);"
                "process.stdout.write(JSON.stringify(r.operation==='list'"
                "?{contractVersion:1,sessions:[{nativeId:'synthetic-failing',title:{source:'native',text:'synthetic'},cwd:null,updatedAtUnixMs:1,createdAtUnixMs:1}],failures:[]}"
                ":{contractVersion:1,error:{code:'provider_read'}}));\n",
                encoding="utf-8",
            )
            (root / "pi_sessions.mjs").write_text(
                "process.stdout.write(JSON.stringify({sessions:[]}));\n", encoding="utf-8"
            )
            module_dir = root / "bindings"
            module_dir.mkdir()
            shutil.copy(args.bindings / "velune_bindings.py", module_dir)
            (module_dir / "libvelune_bindings.dylib").symlink_to(args.library)
            spec = importlib.util.spec_from_file_location("velune_bindings", module_dir / "velune_bindings.py")
            binding = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = binding
            spec.loader.exec_module(binding)
            application = binding.VeluneApplication.open(binding.BindingOptions(
                home_directory=str(root / "app-home"), resources_directory=str(root)))
            runtime = binding.BindingRuntimeInstance(
                enabled=True, id="synthetic", name="Synthetic", type_id="codex-0.159.3",
                gateway_id="default", settings={"binary": str(args.node), "nodeBinary": str(args.node), "agentDir": str(root / "runtime")})
            (root / "runtime" / "sessions").mkdir(parents=True)
            application.upsert_runtime(runtime)
            pi_runtime = binding.BindingRuntimeInstance(
                enabled=True, id="pi", name="Synthetic Pi", type_id="pi-1.0.2",
                gateway_id="default", settings={"binary": str(args.node), "nodeBinary": str(args.node), "agentDir": str(root / "pi-runtime")})
            (root / "pi-runtime").mkdir()
            application.upsert_runtime(pi_runtime)
            listed = application.list()
            assert any(item.id == "synthetic:synthetic-failing" for item in listed.conversations)
            application.select_runtime("pi")
            try:
                application.open_conversation("synthetic", "synthetic:synthetic-failing")
            except binding.BindingError.Diagnostic as error:
                assert error.code == "provider_read" and error.phase == "read"
            else:
                raise AssertionError("expected typed history diagnostic")
            application.shutdown()
        print(json.dumps({
            "acceptance": "PASSED",
            "normalList": True,
            "malformedRecordProjectedWithoutRawError": True,
            "staleReadCode": "session_not_found",
            "realHistoryRead": False,
            "realCredentials": False,
            "applicationBindingDiagnostic": bool(args.bindings and args.library),
            "historyUsesSummaryRuntimeWhileAnotherRuntimeSelected": bool(args.bindings and args.library),
        }))


if __name__ == "__main__":
    main()
