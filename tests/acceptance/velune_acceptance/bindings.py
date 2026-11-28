"""Load the generated Python bindings beside the exact library they belong to."""
from __future__ import annotations

import ast
import hashlib
import importlib
import sys
import types
from pathlib import Path

LIBRARY_NAMES = ("libvelune_bindings.dylib", "libvelune_bindings.so")
MODULES = ("bindings", "application_api", "conversation")


def library(directory: Path) -> Path:
    for name in LIBRARY_NAMES:
        candidate = directory / name
        if candidate.exists():
            return candidate
    raise FileNotFoundError(f"no velune_bindings library in {directory}")


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def _repair(code: str, filename: str) -> str:
    # UniFFI 0.32.2 can put a default before required arguments in generated
    # Command subclasses. Only exported methods are used, so make them
    # keyword-only rather than editing the generator output on disk.
    while True:
        try:
            ast.parse(code, filename=filename)
            return code
        except SyntaxError as error:
            lines = code.splitlines(keepends=True)
            line = lines[error.lineno - 1]
            if (error.msg not in ("parameter without a default follows parameter with a default",
                                  "non-default argument follows default argument")
                    or "def __init__(self, " not in line):
                raise
            lines[error.lineno - 1] = line.replace("def __init__(self, ", "def __init__(self, *, ", 1)
            code = "".join(lines)


def load(directory: Path, scratch: Path, package_name: str) -> types.SimpleNamespace:
    """Import the modules under a private package name so two builds can be
    loaded by different runs without sharing module state."""
    scratch.mkdir(mode=0o700)
    for source in sorted(directory.glob("velune_*.py")):
        (scratch / source.name).write_text(_repair(source.read_text(), str(source)))
    lib = library(directory)
    (scratch / lib.name).symlink_to(lib)
    package = types.ModuleType(package_name)
    package.__path__ = [str(scratch)]
    sys.modules[package_name] = package
    loaded = {name: importlib.import_module(f"{package_name}.velune_{name}") for name in MODULES}
    return types.SimpleNamespace(**loaded, library=lib)
