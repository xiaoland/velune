"""Check development launcher isolation, unavailable backend and bounded local queries."""
import argparse
import contextlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile
import threading


def main():
    root = Path(__file__).resolve().parents[3]
    sys.path.insert(0, str(root / "scripts"))
    helper = runpy.run_path(str(root / "scripts/dev-telemetry.py"), run_name="fixture")
    calls = []

    class Receiver(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            calls.append((self.path, self.headers.get("Authorization")))
            if self.path == "/redirect":
                self.send_response(302); self.send_header("Location", "/must-not-follow"); self.end_headers()
            else:
                self.send_response(200); self.end_headers()
                self.wfile.write(b'{"database":"ok"}' if self.path == "/health" else b"x" * (4 * 1024 * 1024 + 1))

    server = ThreadingHTTPServer(("127.0.0.1", 0), Receiver)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    original = http.client.HTTPConnection
    old_proxy = os.environ.get("HTTP_PROXY")
    try:
        def connection(host, port, **options):
            assert (host, port) == ("127.0.0.1", 30300)
            return original(host, server.server_port, **options)
        http.client.HTTPConnection = connection
        os.environ["HTTP_PROXY"] = "http://127.0.0.1:1"
        assert helper["backend_json"]("/health") == {"database": "ok"}
        for path in ("/redirect", "/large"):
            try:
                helper["backend_json"](path, "CANARY-private-query-token")
            except RuntimeError:
                pass
            else:
                raise AssertionError("redirect or oversized result accepted")
        assert not any(path == "/must-not-follow" for path, _ in calls)
    finally:
        http.client.HTTPConnection = original
        if old_proxy is None:
            os.environ.pop("HTTP_PROXY", None)
        else:
            os.environ["HTTP_PROXY"] = old_proxy
        server.shutdown(); server.server_close()

    with tempfile.TemporaryDirectory(prefix="velune-development-") as temporary:
        directory = Path(temporary)
        executable = directory / "business"
        executable.touch()
        executable.chmod(0o700)
        globals = helper["launch"].__globals__
        globals["BASE"] = directory / "runs"
        globals["ROOT"] = directory
        def unavailable():
            raise RuntimeError("synthetic unavailable backend")
        globals["ready"] = unavailable
        launched = []
        original_popen = subprocess.Popen
        def capture(*args, **options):
            if args[0][0] != str(executable):
                return original_popen(*args, **options)
            assert args == ([str(executable)],)
            environment = options["env"]
            assert "VELUNE_TEST_PRIVATE_CANARY" not in environment
            assert not Path(environment["VELUNE_TELEMETRY_PROFILE"]).exists()
            assert Path(environment["VELUNE_HOME"]).is_dir()
            assert len((environment["VELUNE_HOME"] + "/remote/cli/control.sock").encode()) < 104
            assert options["start_new_session"] is True
            launched.append(True)
            return argparse.Namespace(pid=os.getpid())
        subprocess.Popen = capture
        os.environ["VELUNE_TEST_PRIVATE_CANARY"] = "synthetic-only"
        output = io.StringIO()
        try:
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(io.StringIO()):
                helper["launch"](argparse.Namespace(platform="macos", app=executable))
        finally:
            os.environ.pop("VELUNE_TEST_PRIVATE_CANARY", None)
            subprocess.Popen = original_popen
        manifest = json.loads(output.getvalue())
        assert manifest["attached"] is False
        assert manifest["workspaceRevision"] is None
        assert launched == [True]
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            helper["status"](argparse.Namespace(manifest=Path(manifest["manifestPath"])))
        state = json.loads(output.getvalue())
        assert state["stale"] and state["stateAvailable"] is False and state["belongsToRun"] is None
        shutil.rmtree(Path(manifest["statusPath"]).parents[1])

        mobile_calls = []
        def mobile_run(*args, **kwargs):
            mobile_calls.append(args)
            return ""
        globals["run"] = mobile_run
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(io.StringIO()):
            helper["launch"](argparse.Namespace(platform="android", app=None, adb=Path("adb"), device="synthetic-device", package="app.velune.android"))
        assert not any("install" in call for call in mobile_calls)
        assert any("start" in call and "am" in call for call in mobile_calls)
        assert json.loads(output.getvalue())["appPath"] is None
        assert json.loads(output.getvalue())["attached"] is True
        mobile_calls.clear()
        def blocked_forward(*args, **kwargs):
            if "android" in args and "start" in args and "--adb" in args:
                raise subprocess.CalledProcessError(1, args, stderr="synthetic port conflict")
            return mobile_run(*args, **kwargs)
        globals["run"] = blocked_forward
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(io.StringIO()):
            helper["launch"](argparse.Namespace(platform="android", app=None, adb=Path("adb"), device="synthetic-device", package="app.velune.android"))
        assert json.loads(output.getvalue())["attached"] is False
        assert any("start" in call and "am" in call for call in mobile_calls)
        assert not any("cat >" in str(call) for call in mobile_calls)
    print(json.dumps({"passed": True, "noProxyOrCredentialRedirect": True, "boundedQuery": True,
                      "backendFailureRequestsBusinessLaunch": True, "unrelatedEnvironmentExcluded": True,
                      "shortSocketPath": True, "installedMobileLaunchWithoutInstall": True,
                      "androidForwardFailureStillStartsBusiness": True}))


if __name__ == "__main__":
    main()
