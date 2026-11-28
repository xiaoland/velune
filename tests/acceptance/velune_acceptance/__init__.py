"""Replayable acceptance cases that drive the real application through its
generated Python bindings in an isolated home. See tests/acceptance/README.md."""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path


class Skip(Exception):
    """Raised by a case whose external requirement is unavailable."""


@dataclass
class Context:
    """What the runner gives each case."""

    bindings: object
    isolated: object
    tools: dict[str, Path]
    timeout: float
    checks: dict[str, bool] = field(default_factory=dict)
    facts: dict[str, object] = field(default_factory=dict)

    def tool(self, name: str) -> Path:
        path = self.tools.get(name)
        if path is None:
            raise Skip(f"requires --{name}")
        return path

    def passed(self, check: str) -> None:
        """Record one user-visible guarantee as proven, in order."""
        self.checks[check] = True
