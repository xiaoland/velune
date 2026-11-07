#!/usr/bin/env python3
"""Collect NuGet package license texts for the Windows application bundle."""

from __future__ import annotations

import json
import hashlib
from pathlib import Path, PurePosixPath
import re
import shutil
import sys
import xml.etree.ElementTree as ET


LICENSE_PREFIXES = ("license", "licence", "copying", "notice", "unlicense", "copyright")
ROOT = Path(__file__).resolve().parents[1]
SOURCE_MANIFEST = ROOT / "app/windows/Licenses/NUGET-SOURCES.json"


def package_license_metadata(nuspec: Path) -> tuple[str, str | None, str | None]:
    root = ET.parse(nuspec).getroot()
    metadata = next((node for node in root.iter() if node.tag.endswith("}metadata")), None)
    if metadata is None:
        raise ValueError(f"NuGet package nuspec has no metadata: {nuspec}")
    package_id = next((node.text for node in metadata if node.tag.endswith("}id")), None)
    if not package_id:
        raise ValueError(f"NuGet package nuspec has no id: {nuspec}")
    license_node = next((node for node in metadata if node.tag.endswith("}license")), None)
    license_value = license_node.text or "" if license_node is not None else ""
    license_type = license_node.attrib.get("type") if license_node is not None else None
    repository = next((node for node in metadata if node.tag.endswith("}repository")), None)
    source_url = repository.attrib.get("url") if repository is not None else None
    return package_id, f"{license_type}:{license_value}" if license_type else license_value, source_url


def collect(assets_path: Path, output_dir: Path) -> tuple[int, int]:
    assets = json.loads(assets_path.read_text(encoding="utf-8"))
    source_manifest = json.loads(SOURCE_MANIFEST.read_text(encoding="utf-8"))
    source_packages = {
        (entry["package"].casefold(), entry["version"]): entry
        for entry in source_manifest.get("runtime_packages", [])
    }
    velopack_source = source_manifest.get("velopack_source", {})
    package_folders = [Path(path) for path in assets.get("packageFolders", {})]
    libraries = assets.get("libraries", {})
    runtime_package_keys = set()
    for target in assets.get("targets", {}).values():
        for key, package_assets in target.items():
            library = libraries.get(key, {})
            if library.get("type") != "package":
                continue
            if any(package_assets.get(kind) for kind in ("runtime", "runtimeTargets", "native")) or key.lower().startswith(
                "microsoft.windowsappsdk."
            ):
                runtime_package_keys.add(key)
    packages = sorted(
        (key, libraries[key])
        for key in runtime_package_keys
        if "/" in key and libraries[key].get("path")
    )
    if not packages:
        raise ValueError(f"No restored NuGet packages found in {assets_path}")

    if output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    lines = [
        "# Windows application NuGet package notices",
        "",
        "Package licenses and notices are preserved from the exact NuGet packages in the build's restored dependency graph.",
        "The package lock file is included next to this document.",
        "Build-only Windows SDK compiler packages are omitted because their code is not shipped with the application.",
        "",
    ]
    package_count = 0
    notice_count = 0
    for key, library in packages:
        package_id, version = key.rsplit("/", 1)
        relative = PurePosixPath(library["path"])
        package_dir = next(
            (folder.joinpath(*relative.parts) for folder in package_folders if folder.joinpath(*relative.parts).is_dir()),
            None,
        )
        if package_dir is None:
            raise ValueError(f"Restored NuGet package directory is missing: {key}")
        nuspecs = sorted(package_dir.glob("*.nuspec"))
        if len(nuspecs) != 1:
            raise ValueError(f"Expected one nuspec in restored NuGet package {key}")
        nuspec_id, license_value, source_url = package_license_metadata(nuspecs[0])
        if nuspec_id.lower() != package_id.lower():
            raise ValueError(f"Nuspec identity {nuspec_id!r} does not match lock identity {package_id!r}")
        license_files = sorted(
            path for path in package_dir.rglob("*")
            if path.is_file() and path.name.lower().startswith(LICENSE_PREFIXES)
        )
        source_entry = source_packages.get((package_id.casefold(), version))
        if package_id.casefold() == "velopack":
            source_entry = velopack_source
        if source_entry:
            revision = source_entry.get("revision")
            repository = source_entry.get("repository")
            source_path = source_entry.get("source_path")
            if not revision or not repository:
                raise ValueError(f"Pinned source metadata is incomplete for NuGet package {key}")
            source_url = f"{repository}/tree/{revision}" + (f"/{source_path}" if source_path else "")

        pinned_license = source_entry.get("license_file") if source_entry else None
        if pinned_license:
            license_file = ROOT / pinned_license
            expected_sha = source_entry.get("license_file_sha256")
            if not license_file.is_file() or not expected_sha:
                raise ValueError(f"Pinned license text is missing for NuGet package {key}")
            actual_sha = hashlib.sha256(license_file.read_bytes()).hexdigest()
            if actual_sha != expected_sha:
                raise ValueError(f"Pinned license SHA-256 mismatch for {key}: {actual_sha}")
            existing_hashes = {
                hashlib.sha256(path.read_bytes()).hexdigest()
                for path in license_files
            }
            if actual_sha not in existing_hashes:
                license_files.append(license_file)
        if not license_files and not license_value:
            raise ValueError(f"NuGet package {key} contains no license text or license expression")

        package_out = output_dir / re.sub(r"[^A-Za-z0-9._-]", "_", package_id) / version
        for license_file in license_files:
            try:
                rel = license_file.relative_to(package_dir)
            except ValueError:
                rel = Path(license_file.name)
            destination = package_out.joinpath(*rel.parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(license_file, destination)
            notice_count += 1
        package_count += 1
        line = f"- **{package_id} {version}** — {license_value or 'license text included'}"
        if source_url:
            line += f" — {source_url}"
        if package_id.casefold() == "velopack":
            repository_root = f"{velopack_source['repository']}/tree/{velopack_source['revision']}"
            line += f" — helper sources (`Setup.exe`/`Update.exe`, `Cargo.lock`): {repository_root}/src/bins, {repository_root}/src/lib-rust, {repository_root}/src/l18n"
        lines.append(line)

    (output_dir / "THIRD_PARTY_NOTICES.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return package_count, notice_count


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: scripts/dotnet-licenses.py <project.assets.json> <output directory>", file=sys.stderr)
        return 2
    try:
        packages, notices = collect(Path(sys.argv[1]), Path(sys.argv[2]))
    except (OSError, ValueError, ET.ParseError, json.JSONDecodeError) as error:
        print(f"NuGet license collection failed: {error}", file=sys.stderr)
        return 1
    print(f"{packages} NuGet packages, {notices} license/notice files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
