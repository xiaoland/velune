"""Helpers for driving one opened application through the Python bindings."""
from __future__ import annotations

import time
from collections.abc import Callable


def texts(snapshot) -> list[str]:
    return [block.text for message in snapshot.messages for block in message.blocks if block.is_text()]


def describe(snapshot) -> dict | None:
    if snapshot is None:
        return None
    return {
        "state": snapshot.run_state.name, "access": snapshot.actions.input_access.name,
        "canSend": snapshot.actions.can_send, "canCancel": snapshot.actions.can_cancel,
        "messageCount": len(snapshot.messages), "recentText": [text[:100] for text in texts(snapshot)[-8:]],
        "queueLength": len(snapshot.follow_up.queue),
    }


class Driver:
    """Owns an application handle for one conversation and reopens it on
    restart, so cases read like the user's sequence of actions."""

    def __init__(self, bindings, options, runtime_id: str, timeout: float):
        self.bindings = bindings
        self.options = options
        self.runtime_id = runtime_id
        self.timeout = timeout
        self.key: str | None = None
        self.app = bindings.bindings.VeluneApplication.open(options)

    def read(self):
        return self.app.snapshot(self.runtime_id, self.key).snapshot

    def wait(self, label: str, predicate: Callable, detail: Callable[[], object] = lambda: None):
        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            snapshot = self.read()
            if snapshot is not None and predicate(snapshot):
                return snapshot
            time.sleep(0.05)
        raise TimeoutError(f"{label}; last snapshot={describe(self.read())}; {detail()}")

    def reopen(self) -> None:
        self.close_when_idle()
        self.app = self.bindings.bindings.VeluneApplication.open(self.options)
        self.app.open_conversation(self.runtime_id, self.key)

    def close_when_idle(self) -> None:
        # The automatic title turn can still hold the owner briefly after
        # the conversation itself has published Idle.
        deadline = time.monotonic() + self.timeout
        while True:
            try:
                self.app.shutdown()
                return
            except self.bindings.bindings.BindingError as error:
                if getattr(error, "detail", "") != "runtime is busy" or time.monotonic() >= deadline:
                    raise
                self.read()
                time.sleep(0.05)

    def abandon(self) -> None:
        """Best-effort cleanup after a failure; keeps the original error."""
        error = self.bindings.bindings.BindingError
        if self.key is not None:
            try:
                self.app.cancel(self.runtime_id, self.key)
            except error:
                pass
        try:
            self.app.shutdown()
        except error:
            pass
