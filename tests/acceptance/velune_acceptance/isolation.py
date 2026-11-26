"""A private HOME, Velune state and project for one case, with the process
environment reduced so nothing reaches real configuration or the network."""
from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
RESOURCE_SCRIPTS = REPO / "packages/agent-runtime/resources"
RUNTIME_SUPPORT = REPO / "target/runtime-support/node_modules"


@dataclass(frozen=True)
class Isolated:
    root: Path
    home: Path
    state: Path
    project: Path
    resources: Path

    def dir(self, name: str) -> Path:
        path = self.root / name
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
        return path


def resources(root: Path, explicit: Path | None) -> Path:
    """Use a build's own resources when given; otherwise assemble the runtime
    helper scripts from the source tree and the installed runtime support."""
    target = root / "resources"
    if explicit is not None:
        target.symlink_to(explicit, target_is_directory=True)
        return target
    target.mkdir(mode=0o700)
    for script in RESOURCE_SCRIPTS.iterdir():
        if script.is_file():
            (target / script.name).symlink_to(script)
    if RUNTIME_SUPPORT.is_dir():
        (target / "node_modules").symlink_to(RUNTIME_SUPPORT, target_is_directory=True)
    return target


def enter(root: Path, explicit_resources: Path | None, extra_environment: dict[str, str] | None = None) -> Isolated:
    """Create the directories and replace the process environment.

    Proxies point at a closed port so a misconfigured client fails instead of
    reaching the network; only loopback addresses bypass them.
    """
    home, state, project = (root / name for name in ("home", "state", "project"))
    for directory in (home, state, project):
        directory.mkdir(mode=0o700)
    isolated = Isolated(root, home, state, project, resources(root, explicit_resources))
    inherited_tmp = os.environ.get("TMPDIR", "/tmp")
    os.environ.clear()
    os.environ.update(HOME=str(home), VELUNE_HOME=str(state), TMPDIR=inherited_tmp,
                      PATH="/usr/bin:/bin:/usr/sbin:/sbin",
                      DISABLE_TELEMETRY="1", DISABLE_ERROR_REPORTING="1")
    for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"):
        os.environ[key] = os.environ[key.lower()] = "http://127.0.0.1:1"
    os.environ["NO_PROXY"] = os.environ["no_proxy"] = "localhost,127.0.0.1"
    os.environ.update(extra_environment or {})
    return isolated
