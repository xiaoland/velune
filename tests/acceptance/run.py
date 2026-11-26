#!/usr/bin/env python3
"""Run the replayable acceptance cases against one build of the bindings.

Each case runs in its own process with a private HOME and Velune state, and
the run writes a manifest under target/verification/acceptance/. See
tests/acceptance/README.md.
"""
from __future__ import annotations

import argparse
import datetime
import importlib
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import traceback
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
CASES = HERE / "cases"
DEFAULT_BINDINGS = REPO / "target/acceptance/bindings"
RESULTS = REPO / "target/verification/acceptance"
TOOLS = ("codex", "claude", "pi")


def discover() -> list[str]:
    return sorted(path.stem for path in CASES.glob("*.py") if not path.name.startswith("_"))


def git(*args: str) -> str:
    return subprocess.run(["git", "-C", str(REPO), *args], capture_output=True, text=True, check=True).stdout.strip()


def tool_version(path: Path) -> str | None:
    with tempfile.TemporaryDirectory(prefix="velune-acceptance-version-") as home:
        try:
            done = subprocess.run([str(path), "--version"], capture_output=True, text=True, timeout=20,
                                  env={"HOME": home, "PATH": "/usr/bin:/bin"})
        except (OSError, subprocess.TimeoutExpired):
            return None
    # CLIs may print warnings first; record the line that carries a version.
    lines = [line.strip() for line in (done.stdout + "\n" + done.stderr).splitlines() if line.strip()]
    return next((line for line in lines if re.search(r"\d+\.\d+", line)), lines[0] if lines else None)


def build(out: Path) -> None:
    """Build the host bindings library and generate matching Python modules."""
    subprocess.run(["cargo", "build", "--locked", "-p", "velune-bindings"], cwd=REPO, check=True)
    library = next(path for name in ("libvelune_bindings.dylib", "libvelune_bindings.so")
                   if (path := REPO / "target/host-rust/debug" / name).exists())
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    subprocess.run(["cargo", "run", "--locked", "--target-dir", "target/build-tools", "-p", "velune-build-tools",
                    "--bin", "velune-bindgen", "--", "generate", "--library", str(library), "--no-format",
                    "--language", "python", "--out-dir", str(out)], cwd=REPO, check=True)
    shutil.copy2(library, out / library.name)


def worker(name: str, config: dict, result_path: Path) -> int:
    """Run one case in this process. Called by the parent with -I."""
    sys.path.insert(0, str(HERE))
    from velune_acceptance import Context, Skip, bindings, isolation

    result: dict = {"case": name, "status": "failed", "checks": {}, "facts": {}}
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix=f"velune-acceptance-{name}-") as temporary:
            root = Path(temporary).resolve()
            resources = Path(config["resources"]) if config["resources"] else None
            isolated = isolation.enter(root, resources)
            loaded = bindings.load(Path(config["bindings"]), root / "bindings", f"acceptance_{name}")
            ctx = Context(bindings=loaded, isolated=isolated, timeout=config["timeout"],
                          tools={key: Path(value) for key, value in config["tools"].items()})
            result["checks"], result["facts"] = ctx.checks, ctx.facts
            module = importlib.import_module(f"cases.{name}")
            missing = [tool for tool in getattr(module, "REQUIRES", ()) if tool not in ctx.tools]
            if missing:
                raise Skip("requires " + ", ".join(f"--{tool}" for tool in missing))
            module.run(ctx)
            result["status"] = "passed"
    except Skip as skipped:
        result["status"], result["reason"] = "skipped", str(skipped)
    except BaseException as error:  # Report every failure in the manifest.
        result["error"] = f"{type(error).__name__}: {error}"
        result["traceback"] = traceback.format_exc()
    result["seconds"] = round(time.monotonic() - started, 2)
    result_path.write_text(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result["status"] != "failed" else 1


def main() -> int:
    if len(sys.argv) == 5 and sys.argv[1] == "_worker":
        return worker(sys.argv[2], json.loads(sys.argv[3]), Path(sys.argv[4]))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cases", nargs="*", help=f"cases to run (default: all of {', '.join(discover())})")
    parser.add_argument("--build", action="store_true", help="build the bindings library and Python modules first")
    parser.add_argument("--bindings", type=Path, default=DEFAULT_BINDINGS,
                        help="directory with matching velune_*.py modules and the bindings library")
    parser.add_argument("--resources", type=Path, help="a build's runtime resources (default: source helpers)")
    for tool in TOOLS:
        parser.add_argument(f"--{tool}", type=Path, help=f"absolute path of an installed {tool} CLI")
    parser.add_argument("--timeout", type=float, default=15.0, help="per-step timeout in seconds")
    parser.add_argument("--label", default="", help="free-form build label recorded in the manifest")
    args = parser.parse_args()

    known = discover()
    unknown = sorted(set(args.cases) - set(known))
    if unknown:
        parser.error(f"unknown cases: {', '.join(unknown)}")
    if args.build:
        build(args.bindings)
    sys.path.insert(0, str(HERE))
    from velune_acceptance import bindings

    tools = {tool: str(getattr(args, tool).expanduser().resolve(strict=True))
             for tool in TOOLS if getattr(args, tool) is not None}
    config = {"bindings": str(args.bindings.expanduser().resolve(strict=True)),
              "resources": str(args.resources.expanduser().resolve(strict=True)) if args.resources else None,
              "tools": tools, "timeout": args.timeout}

    run_id = datetime.datetime.now(datetime.UTC).strftime("%Y%m%dT%H%M%SZ") + "-" + secrets.token_hex(3)
    out = RESULTS / run_id
    out.mkdir(parents=True)
    manifest = {
        "run": run_id,
        "source": {"head": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain")), "label": args.label},
        "bindings_sha256": bindings.digest(bindings.library(Path(config["bindings"]))),
        "tools": {tool: {"path": path, "version": tool_version(Path(path))} for tool, path in tools.items()},
        "cases": [],
    }
    for name in args.cases or known:
        result_path = out / f"{name}.json"
        log_path = out / f"{name}.log"
        with log_path.open("w") as log:
            subprocess.run([sys.executable, "-I", str(Path(__file__).resolve()), "_worker", name,
                            json.dumps(config), str(result_path)],
                           cwd=HERE, stdout=log, stderr=subprocess.STDOUT, env={"PATH": os.environ.get("PATH", "")})
        result = json.loads(result_path.read_text()) if result_path.exists() else {
            "case": name, "status": "failed", "error": f"worker produced no result; see {log_path.name}"}
        manifest["cases"].append({key: value for key, value in result.items() if key != "traceback"})
        mark = {"passed": "PASS", "skipped": "SKIP", "failed": "FAIL"}[result["status"]]
        print(f"{mark} {name}" + (f" — {result.get('reason') or result.get('error')}" if mark != "PASS" else
                                  f" ({len(result.get('checks', {}))} checks, {result.get('seconds')}s)"))
    (out / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2))
    print(f"manifest: {out / 'manifest.json'}")
    return 1 if any(case["status"] == "failed" for case in manifest["cases"]) else 0


if __name__ == "__main__":
    sys.exit(main())
