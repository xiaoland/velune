#!/usr/bin/env python3
"""手动检查 Claude 历史 helper 的分支和来源边界；只使用临时合成文件。"""
import json
import pathlib
import subprocess
import tempfile
import uuid

REPO = pathlib.Path(__file__).resolve().parents[1]
HELPER = REPO / "packages/agent-runtime/resources/claude_history.py"


def main():
    with tempfile.TemporaryDirectory(prefix="velune-runtime-history-") as temporary:
        root = pathlib.Path(temporary)
        projects = root / "projects"
        directory = projects / "synthetic-project"
        directory.mkdir(parents=True)
        native_id = str(uuid.uuid4())
        path = directory / f"{native_id}.jsonl"
        rows = [
            {"type": "user", "uuid": "user", "parentUuid": None, "cwd": str(root), "timestamp": "2026-10-10T00:00:00Z", "message": {"content": "合成问题"}},
            {"type": "assistant", "uuid": "abandoned", "parentUuid": "user", "message": {"content": [{"type": "text", "text": "旧分支"}]}},
            {"type": "assistant", "uuid": "current", "parentUuid": "user", "timestamp": "2026-10-10T00:00:01Z", "message": {"content": [{"type": "text", "text": "当前分支"}]}},
            {"type": "custom-title", "customTitle": "合成标题"},
        ]
        path.write_text("".join(json.dumps(row) + "\n" for row in rows))
        # This synthetic auth sentinel must never become a history candidate.
        (root / "credentials.json").write_text("not session JSON")
        subagents = directory / "subagents"
        subagents.mkdir()
        (subagents / f"{uuid.uuid4()}.jsonl").write_text("not session JSON")

        def invoke(operation, selected=None, **extra):
            request = {"operation": operation, "provider": "claude-code", "home": str(root), "roots": [str(projects)], "runtimeId": "claude", "nativeId": selected, **extra}
            result = subprocess.run(["python3", str(HELPER)], input=json.dumps(request), text=True, capture_output=True)
            assert result.returncode == 0, result.stdout + result.stderr
            return json.loads(result.stdout)

        indexed = invoke("index")
        assert indexed["contractVersion"] == 1
        assert [entry["nativeId"] for entry in indexed["entries"]] == [native_id]
        listed = invoke("summaries", nativeIds=[native_id])
        assert len(listed["sessions"]) == 1
        assert listed["sessions"][0]["title"] == {"source": "native", "text": "合成标题"}
        read = invoke("read", native_id)["history"]
        assert [message["id"] for message in read["messages"]] == ["user", "current"]
        assert read["createdAtUnixMs"] == 1791590400000
        assert invoke("read", str(uuid.uuid4()))["error"]["code"] == "session_not_found"
    print("PASS Claude 合成历史：当前分支、标题、日期、缺失会话与来源范围")


if __name__ == "__main__":
    main()
