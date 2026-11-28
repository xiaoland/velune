"""claude_background_follow_up with the turns routed through the Velune
gateway to a managed provider instead of the CLI's native credentials."""
from __future__ import annotations

from cases import claude_background_follow_up as native
from velune_acceptance import Context

REQUIRES = native.REQUIRES


def run(ctx: Context) -> None:
    native.run(ctx, managed=True)
