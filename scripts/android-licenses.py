#!/usr/bin/env python3
"""Writes the licence notices of the Rust crates linked into the Android library.

MIT, Apache-2.0, BSD and similar licences ask that their notices travel with
binary copies, so the APK carries this file and shows it under
“关于与开源许可”. Reads `cargo metadata` for the same target and features as
build-android-native.sh; workspace crates are Velune's own and are skipped.

Usage: scripts/android-licenses.py <features> <output file> [Rust target]
"""
import hashlib
import json
import os
import subprocess
import sys

TARGET = sys.argv[3] if len(sys.argv) > 3 else "aarch64-linux-android"
LICENSE_PREFIXES = ("license", "licence", "copying", "notice", "unlicense")


def standard_text(name: str) -> str:
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    with open(os.path.join(root, "app/android/licenses", name + ".txt"), encoding="utf-8") as file:
        return file.read().strip()


def main() -> None:
    features, output = sys.argv[1], sys.argv[2]
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", TARGET,
         "--manifest-path", os.path.join(root, "packages/bindings/Cargo.toml"),
         "--no-default-features", "--features", features],
        check=True, capture_output=True, text=True,
    ).stdout)
    packages = {package["id"]: package for package in metadata["packages"]}
    workspace = set(metadata["workspace_members"])
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    start = next(id for id in workspace if packages[id]["name"] == "velune-bindings")

    # Normal dependencies only: build scripts and dev-dependencies never reach the APK.
    linked, pending = set(), [start]
    while pending:
        id = pending.pop()
        if id in linked:
            continue
        linked.add(id)
        for dep in nodes[id]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                pending.append(dep["pkg"])

    crates = sorted((packages[id] for id in linked - workspace), key=lambda p: (p["name"], p["version"]))
    texts: dict[str, tuple[str, list[str]]] = {}
    missing = []
    for crate in crates:
        directory = os.path.dirname(crate["manifest_path"])
        files = sorted(name for name in os.listdir(directory) if name.lower().startswith(LICENSE_PREFIXES) and os.path.isfile(os.path.join(directory, name)))
        if not files:
            missing.append(crate)
        for name in files:
            with open(os.path.join(directory, name), encoding="utf-8", errors="replace") as file:
                text = file.read().strip()
            digest = hashlib.sha256(text.encode()).hexdigest()
            texts.setdefault(digest, (text, []))[1].append(f"{crate['name']} {crate['version']}")

    lines = [
        "Rust 组件许可",
        "",
        f"{TARGET} 原生库链接了以下 {len(crates)} 个 Rust crate。",
        "每个 crate 后注明其声明的许可；下方附各 crate 随源码分发的许可全文（相同全文只列一次）。",
        "",
    ]
    for crate in crates:
        source = crate.get("repository") or ""
        lines.append(f"- {crate['name']} {crate['version']} — {crate.get('license') or crate.get('license_file') or '未声明'}" + (f" — {source}" if source else ""))
    # Crates published without a licence file get the standard text of the licence they declare;
    # of alternatives (“MIT OR Apache-2.0”), Apache-2.0 is used, as its text needs no holder.
    standard: dict[str, list[str]] = {}
    for crate in missing:
        expression = crate.get("license") or ""
        name = next((spdx for spdx in ("Apache-2.0", "MPL-2.0") if spdx in expression), "MIT" if "MIT" in expression else None)
        if name is None:
            sys.exit(f"{crate['name']} {crate['version']} declares {expression!r} and ships no licence file; add its text.")
        if name == "MIT":
            holders = ", ".join(crate.get("authors") or [f"the {crate['name']} authors"])
            text = standard_text("MIT").replace("{holders}", holders)
            texts.setdefault(hashlib.sha256(text.encode()).hexdigest(), (text, []))[1].append(f"{crate['name']} {crate['version']}")
        else:
            standard.setdefault(name, []).append(f"{crate['name']} {crate['version']}")
    for name, users in standard.items():
        texts.setdefault(name, (standard_text(name), users))
    for text, users in texts.values():
        lines += ["", "=" * 72, "适用于：" + "，".join(users), "=" * 72, "", text]
    os.makedirs(os.path.dirname(os.path.abspath(output)), exist_ok=True)
    with open(output, "w", encoding="utf-8") as file:
        file.write("\n".join(lines) + "\n")
    print(f"{len(crates)} crates, {len(texts)} distinct licence texts -> {output}")


if __name__ == "__main__":
    main()
