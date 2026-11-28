"""Describe the actual acceptance inputs, separately from the runner checkout."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

from . import bindings, isolation


def file_identity(path: Path) -> dict:
    return {"path": str(path.resolve()), "sha256": bindings.digest(path)}


def directory_identity(path: Path) -> dict:
    """Hash paths and content, including dependencies reached through symlinks."""
    digest = hashlib.sha256()
    count = 0
    visited = set()
    for directory, folders, files in os.walk(path, followlinks=True):
        base = Path(directory)
        relative = base.relative_to(path).as_posix()
        resolved = base.resolve()
        if resolved in visited:
            # A repeated directory may be a package alias or a cycle. Its
            # target is already part of the input; record the alias and stop.
            digest.update(json.dumps([relative, "directory-link", str(resolved)]).encode() + b"\n")
            folders[:] = []
            continue
        visited.add(resolved)
        folders.sort()
        for name in sorted(files):
            source = base / name
            digest.update(json.dumps([(source.relative_to(path)).as_posix(), bindings.digest(source)]).encode() + b"\n")
            count += 1
    return {"path": str(path.resolve()), "sha256": digest.hexdigest(), "files": count}


def generation_receipt(directory: Path, library: Path, modules: dict) -> dict | None:
    path = directory / ".velune-bindings.json"
    if not path.is_file():
        return None
    # Check the pair described by build-support.py's existing receipt. Its
    # generator/config may belong to another revision, so do not compare it
    # with the runner checkout when accepting --bindings from an older build.
    receipt = json.loads(path.read_text())
    files = receipt.get("files", {}) if isinstance(receipt, dict) else {}
    if (not isinstance(files, dict) or not files
            or receipt.get("inputs", {}).get("library") != bindings.digest(library)
            or not modules.keys() <= files.keys()
            or any(Path(name).is_absolute() or ".." in Path(name).parts
                   or not (directory / name).is_file()
                   or bindings.digest(directory / name) != expected for name, expected in files.items())):
        raise ValueError(f"bindings generation receipt does not match the library or generated files in {directory}; rebuild this pair")
    return {**file_identity(path), "receipt": receipt}


def capture(directory: Path, resources: Path | None, source: Path) -> dict:
    library = bindings.library(directory)
    modules = {path.name: file_identity(path) for path in sorted(directory.glob("velune_*.py"))}
    generated = generation_receipt(directory, library, modules)
    sources = [source / "run.py", *sorted((source / "cases").glob("*.py")),
               *sorted((source / "velune_acceptance").glob("*.py"))]
    actual_resources = ({"bundle": resources} if resources is not None else isolation.resource_sources())
    return {
        "bindings": {"directory": str(directory.resolve()), "library": file_identity(library),
                     "python_modules": modules, "generation": generated},
        "acceptance": {path.relative_to(source).as_posix(): file_identity(path) for path in sources},
        "resources": {"mode": "explicit" if resources is not None else "source_helpers",
                      "entries": {name: directory_identity(path) if path.is_dir() else file_identity(path)
                                  for name, path in sorted(actual_resources.items())}},
    }
