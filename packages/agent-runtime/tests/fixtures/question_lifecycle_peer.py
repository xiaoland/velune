#!/usr/bin/python3
"""Synthetic native request peer with no model or account access."""

import json
import sys


if "--version" in sys.argv:
    print("codex-cli 0.166.0")
    sys.exit(0)

for line in sys.stdin:
    request = json.loads(line)
    if "id" in request and "method" in request:
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": {}}), flush=True)
