#!/usr/bin/python3
import json
import pathlib
import sys

root = pathlib.Path(__file__).parent
with (root / "calls").open("a") as calls:
    if sys.argv[1] == "--input-type=module":
        assert sys.argv[-1] == str(root / "pi-launcher")
        calls.write("resolve\n")
        print(json.dumps({"binary": str(root / "pi.js")}))
    else:
        assert sys.argv[1:] == [str(root / "pi.js"), "--version"]
        calls.write("version\n")
        if (root / "reject").exists():
            sys.exit(1)
        print("1.4.0")
