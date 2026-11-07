#!/usr/bin/env python3
"""Create the corresponding source archive for an exact release tag and commit."""

from __future__ import annotations

import argparse
import base64
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[1]
RELAYKIT_GO_MOD = Path("packages/ai-provider/relaykit-bridge/go.mod")
RELAYKIT_MODULE = "github.com/QuantumNous/new-api/relaykit"
GO_MINIMUM = (1, 25, 1)
SUPPORTED_PLATFORMS = {"macos", "windows", "android", "ios"}
SOURCE_BUILD_OUTPUTS = {
    ".git",
    ".build",
    ".swiftpm",
    "DerivedData",
    "dist",
    "target",
    "app/android/.gradle",
    "app/android/build",
    "app/android/.kotlin",
    "app/android/app/build",
    "app/windows/bin",
    "app/windows/obj",
    "app/ios/xcuserdata",
    "app/ios/Velune.xcodeproj/xcuserdata",
    "app/ios/Velune.xcodeproj/project.xcworkspace/xcuserdata",
    "packages/agent-runtime/runtime-support/node_modules",
}


class BundleError(RuntimeError):
    pass


def run(*args: str, cwd: Path = ROOT, env: dict[str, str] | None = None) -> str:
    try:
        result = subprocess.run(args, cwd=cwd, env=env, check=True, text=True, capture_output=True)
    except FileNotFoundError as error:
        raise BundleError(f"Required command not found: {args[0]}") from error
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() or error.stdout.strip()
        raise BundleError(f"Command failed: {' '.join(args)}\n{detail}") from error
    return result.stdout.strip()


def module_version(go_mod: str) -> str:
    match = re.search(rf"(?m)^\s*(?:require\s+)?{re.escape(RELAYKIT_MODULE)}\s+(v\S+)\s*$", go_mod)
    if not match:
        raise BundleError(f"Could not find the pinned {RELAYKIT_MODULE} version in go.mod")
    version = match.group(1)
    pseudo = re.fullmatch(r"v\d+\.\d+\.\d+-0\.\d{14}-([0-9a-f]{12})", version)
    if not pseudo:
        raise BundleError(f"Expected a commit-pinned Go pseudo-version, found {version}")
    return version


def parse_go_version(output: str) -> tuple[int, int, int]:
    match = re.search(r"\bgo(\d+)\.(\d+)(?:\.(\d+))?\b", output)
    if not match:
        raise BundleError(f"Could not parse Go version from: {output}")
    return tuple(int(part or 0) for part in match.groups())  # type: ignore[return-value]


def safe_extract_git_archive(archive: Path, destination: Path, root_name: str) -> Path:
    expected_root = PurePosixPath(root_name)
    source_root = destination / root_name
    with tarfile.open(archive, "r:") as source:
        for member in source:
            member_path = PurePosixPath(member.name)
            if member_path.is_absolute() or ".." in member_path.parts or not member_path.parts:
                raise BundleError(f"Unsafe path in git archive: {member.name}")
            if member_path.parts[0] != root_name:
                raise BundleError(f"Unexpected path in git archive: {member.name}")
            relative = PurePosixPath(*member_path.parts[1:])
            target = destination.joinpath(*member_path.parts)

            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                target.chmod(member.mode & 0o777)
            elif member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                stream = source.extractfile(member)
                if stream is None:
                    raise BundleError(f"Cannot read archived file: {member.name}")
                with stream, target.open("wb") as output:
                    shutil.copyfileobj(stream, output)
                target.chmod(member.mode & 0o777)
            elif member.issym():
                link = PurePosixPath(member.linkname)
                if link.is_absolute():
                    raise BundleError(f"Absolute symlink in git archive: {member.name}")
                stack = list(relative.parts[:-1])
                for component in link.parts:
                    if component in ("", "."):
                        continue
                    if component == "..":
                        if not stack:
                            raise BundleError(f"Symlink escapes source root: {member.name}")
                        stack.pop()
                    else:
                        stack.append(component)
                if (expected_root / PurePosixPath(*stack)).parts[0] != root_name:
                    raise BundleError(f"Symlink escapes source root: {member.name}")
                target.parent.mkdir(parents=True, exist_ok=True)
                target.symlink_to(member.linkname)
            else:
                raise BundleError(f"Unsupported entry in git archive: {member.name}")

    if not source_root.is_dir():
        raise BundleError("Git archive did not contain its expected source root")
    return source_root


def tree_digest(root: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix().encode()
        digest.update(relative + b"\0")
        if path.is_symlink():
            digest.update(b"L" + os.readlink(path).encode())
        elif path.is_file():
            digest.update(b"F" + path.read_bytes())
        elif path.is_dir():
            digest.update(b"D")
    return digest.hexdigest()


def source_tree_digest(root: Path) -> str:
    digest = hashlib.sha256()
    ignored = {PurePosixPath(path).parts for path in SOURCE_BUILD_OUTPUTS}
    for path in sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix()):
        relative = PurePosixPath(path.relative_to(root).as_posix())
        if relative.as_posix() == "SOURCE-ARCHIVE.json":
            continue
        if any(relative.parts[: len(prefix)] == prefix for prefix in ignored):
            continue
        digest.update(relative.as_posix().encode() + b"\0")
        if path.is_symlink():
            digest.update(b"L" + os.readlink(path).encode())
        elif path.is_file():
            digest.update(b"F" + path.read_bytes())
        elif path.is_dir():
            digest.update(b"D")
    return digest.hexdigest()


def vendor_cargo_sources(source_root: Path) -> str:
    cargo_config = source_root / ".cargo" / "config.toml"
    existing_config = cargo_config.read_text(encoding="utf-8") if cargo_config.exists() else ""
    if re.search(r"(?m)^\s*\[source\.(?:crates-io|vendored-sources)\]\s*$", existing_config):
        raise BundleError("Cannot add archive vendoring because .cargo/config.toml already configures Cargo sources")
    configuration = run(
        "cargo",
        "vendor",
        "--locked",
        "--versioned-dirs",
        "--manifest-path",
        "Cargo.toml",
        "vendor",
        cwd=source_root,
    )
    vendor_dir = source_root / "vendor"
    if not vendor_dir.is_dir() or not any(vendor_dir.iterdir()):
        raise BundleError("Cargo vendoring did not produce the locked Rust dependency sources")
    cargo_config.parent.mkdir(parents=True, exist_ok=True)
    cargo_config.write_text(
        existing_config.rstrip() + "\n\n" + configuration.rstrip() + "\n",
        encoding="utf-8",
    )
    return tree_digest(vendor_dir)


def vendor_android_sources(source_root: Path) -> dict[str, object]:
    lockfile = source_root / "app/android/app/gradle.lockfile"
    if not lockfile.is_file():
        raise BundleError("Android Gradle lockfile is required to archive its linked dependency sources")

    sources_root = source_root / "android-sources"
    entries: list[dict[str, object]] = []
    no_source_artifacts = {
        "androidx.compose:compose-bom": "BOM metadata only; contains no linked code",
        "io.opentelemetry:opentelemetry-bom": "BOM metadata only; contains no linked code",
        "org.jetbrains.kotlin:kotlin-bom": "BOM metadata only; contains no linked code",
        "org.jetbrains.kotlinx:kotlinx-coroutines-bom": "BOM metadata only; contains no linked code",
        "org.jetbrains.kotlinx:kotlinx-serialization-bom": "BOM metadata only; contains no linked code",
        "org.jetbrains.kotlin:kotlin-stdlib-common": "multiplatform metadata; sources are included in kotlin-stdlib",
    }
    coordinates: list[tuple[str, str, str]] = []
    for line in lockfile.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#") or "=" not in line:
            continue
        key = line.split("=", 1)[0]
        pieces = key.rsplit(":", 2)
        if len(pieces) != 3:
            if key == "empty":
                continue
            raise BundleError(f"Unrecognized Gradle lock entry: {line}")
        group, name, version = pieces
        coordinates.append((group, name, version))

    unique_coordinates = sorted(set(coordinates))
    for group, name, version in unique_coordinates:
        coordinate = f"{group}:{name}:{version}"
        if f"{group}:{name}" in no_source_artifacts:
            continue
        artifact = f"{name}-{version}-sources.jar"
        relative = Path(group.replace(".", "/")) / name / version / artifact
        destination = sources_root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        path = relative.as_posix()
        source_url = None
        errors: list[str] = []
        for repository in (
            "https://dl.google.com/dl/android/maven2/",
            "https://repo.maven.apache.org/maven2/",
        ):
            url = repository + path
            try:
                with urlopen(Request(url, headers={"User-Agent": "Velune-source-bundle/1"}), timeout=45) as response:
                    payload = response.read()
                if not payload:
                    errors.append(f"empty source artifact at {url}")
                    continue
                destination.write_bytes(payload)
                source_url = url
                break
            except (HTTPError, URLError, TimeoutError) as error:
                errors.append(f"{url}: {error}")
        if source_url is None:
            raise BundleError(
                f"Could not retrieve the locked Android source artifact for {coordinate}; "
                + "; ".join(errors)
            )
        entries.append(
            {
                "coordinate": coordinate,
                "source_url": source_url,
                "archive_path": (Path("android-sources") / relative).as_posix(),
                "sha256": hashlib.sha256(destination.read_bytes()).hexdigest(),
            }
        )

    manifest = {
        "lockfile": "app/android/app/gradle.lockfile",
        "source_artifact_count": len(entries),
        "excluded_metadata": no_source_artifacts,
        "artifacts": entries,
    }
    (source_root / "ANDROID-SOURCES.json").write_text(
        json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    return {
        "source_artifact_count": len(entries),
        "source_tree_sha256": tree_digest(sources_root),
        "manifest_sha256": hashlib.sha256((source_root / "ANDROID-SOURCES.json").read_bytes()).hexdigest(),
    }


def vendor_swift_sources(source_root: Path) -> dict[str, object]:
    resolved_files = [
        source_root / "Package.resolved",
        source_root / "app/ios/Velune.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved",
    ]
    pins: dict[tuple[str, str], dict[str, str]] = {}
    for resolved_file in resolved_files:
        if not resolved_file.is_file():
            continue
        try:
            state = json.loads(resolved_file.read_text(encoding="utf-8"))
            package_pins = state["pins"]
            for pin in package_pins:
                location = pin["location"].removesuffix(".git")
                revision = pin["state"]["revision"]
                identity = pin.get("identity") or location.rstrip("/").split("/")[-1]
                pins[(location, revision)] = {"identity": identity, "version": pin["state"].get("version", "")}
        except (KeyError, TypeError, json.JSONDecodeError) as error:
            raise BundleError(f"Could not parse Swift Package.resolved {resolved_file}: {error}") from error

    swift_root = source_root / "swift-sources"
    entries: list[dict[str, str]] = []
    for (location, revision), package in sorted(pins.items()):
        if not location.startswith("https://github.com/"):
            raise BundleError(f"Unsupported Swift package source URL: {location}")
        identity = re.sub(r"[^A-Za-z0-9._-]", "_", package["identity"])
        filename = f"{identity}-{revision}.tar.gz"
        destination = swift_root / filename
        destination.parent.mkdir(parents=True, exist_ok=True)
        source_url = f"{location}/archive/{revision}.tar.gz"
        try:
            with urlopen(Request(source_url, headers={"User-Agent": "Velune-source-bundle/1"}), timeout=90) as response:
                payload = response.read()
        except (HTTPError, URLError, TimeoutError) as error:
            raise BundleError(f"Could not retrieve Swift source for {package['identity']} at {revision}: {error}") from error
        if not payload:
            raise BundleError(f"Swift source archive is empty for {package['identity']} at {revision}")
        destination.write_bytes(payload)
        entries.append(
            {
                "identity": package["identity"],
                "version": package["version"],
                "repository": location,
                "revision": revision,
                "source_url": source_url,
                "archive_path": (Path("swift-sources") / filename).as_posix(),
                "sha256": hashlib.sha256(payload).hexdigest(),
            }
        )
    manifest = {"pin_count": len(entries), "packages": entries}
    (source_root / "SWIFT-SOURCES.json").write_text(
        json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    return {
        "pin_count": len(entries),
        "source_tree_sha256": tree_digest(swift_root),
        "manifest_sha256": hashlib.sha256((source_root / "SWIFT-SOURCES.json").read_bytes()).hexdigest(),
    }


def vendor_npm_sources(source_root: Path) -> dict[str, object]:
    lockfile = source_root / "packages/agent-runtime/runtime-support/package-lock.json"
    lock = json.loads(lockfile.read_text(encoding="utf-8"))
    package_sources: list[tuple[str, dict[str, object]]] = []
    for path, package in lock.get("packages", {}).items():
        if not path or not path.startswith("node_modules/") or not package.get("resolved"):
            continue
        if not package.get("integrity"):
            raise BundleError(f"npm source package lacks an integrity value: {path}")
        package_sources.append((path, package))

    npm_root = source_root / "npm-sources"
    entries: list[dict[str, str]] = []
    for package_path, package in sorted(package_sources):
        name = str(package_path.removeprefix("node_modules/")).replace("/", "_").replace("@", "")
        filename = f"{name}-{package['version']}.tgz"
        destination = npm_root / filename
        destination.parent.mkdir(parents=True, exist_ok=True)
        source_url = str(package["resolved"])
        try:
            with urlopen(Request(source_url, headers={"User-Agent": "Velune-source-bundle/1"}), timeout=60) as response:
                payload = response.read()
        except (HTTPError, URLError, TimeoutError) as error:
            raise BundleError(f"Could not retrieve npm source for {package_path}: {error}") from error
        if not payload:
            raise BundleError(f"npm source archive is empty for {package_path}")
        integrity = str(package["integrity"])
        algorithm, encoded = integrity.split("-", 1)
        digest = hashlib.new(algorithm, payload).digest()
        if base64.b64encode(digest).decode("ascii") != encoded:
            raise BundleError(f"npm source integrity does not match package-lock.json for {package_path}")
        destination.write_bytes(payload)
        entries.append(
            {
                "package": package_path.removeprefix("node_modules/"),
                "version": str(package["version"]),
                "source_url": source_url,
                "integrity": integrity,
                "archive_path": (Path("npm-sources") / filename).as_posix(),
                "sha256": hashlib.sha256(payload).hexdigest(),
            }
        )
    manifest = {"package_count": len(entries), "packages": entries}
    (source_root / "NPM-SOURCES.json").write_text(
        json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    return {
        "package_count": len(entries),
        "source_tree_sha256": tree_digest(npm_root),
        "manifest_sha256": hashlib.sha256((source_root / "NPM-SOURCES.json").read_bytes()).hexdigest(),
    }


def vendor_mermaid_sources(source_root: Path) -> dict[str, object]:
    metadata_path = source_root / "app/mac/Licenses/Mermaid/SOURCES.json"
    try:
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BundleError(f"Could not read the pinned Mermaid source map metadata: {error}") from error
    if (
        metadata.get("package") != "mermaid"
        or metadata.get("version") != "11.12.0"
        or metadata.get("bundled_file") != "app/mac/Assets/Artifacts/mermaid.min.js"
        or metadata.get("source_map_path") != "package/dist/mermaid.min.js.map"
    ):
        raise BundleError("Unexpected Mermaid source map metadata")

    url = metadata.get("source_url")
    integrity = metadata.get("integrity")
    if url != "https://registry.npmjs.org/mermaid/-/mermaid-11.12.0.tgz" or not isinstance(integrity, str):
        raise BundleError("Mermaid source map metadata must name its exact npm package tarball and integrity")
    try:
        with urlopen(Request(url, headers={"User-Agent": "Velune-source-bundle/1"}), timeout=90) as response:
            payload = response.read()
    except (HTTPError, URLError, TimeoutError) as error:
        raise BundleError(f"Could not retrieve the exact Mermaid source package: {error}") from error
    algorithm, encoded = integrity.split("-", 1)
    if algorithm != "sha512":
        raise BundleError(f"Unsupported Mermaid npm integrity algorithm: {algorithm}")
    if base64.b64encode(hashlib.new(algorithm, payload).digest()).decode("ascii") != encoded:
        raise BundleError("Mermaid npm tarball integrity does not match app/mac/Licenses/Mermaid/SOURCES.json")

    expected_asset = source_root / "app/mac/Assets/Artifacts/mermaid.min.js"
    if hashlib.sha256(expected_asset.read_bytes()).hexdigest() != metadata.get("bundled_file_sha256"):
        raise BundleError("The tracked Mermaid bundle no longer matches its pinned package metadata")

    try:
        with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as package:
            asset = package.extractfile("package/dist/mermaid.min.js")
            source_map = package.extractfile(metadata["source_map_path"])
            if asset is None or source_map is None:
                raise BundleError("Mermaid npm tarball is missing its bundled asset or source map")
            asset_bytes = asset.read()
            source_map_bytes = source_map.read()
    except (KeyError, OSError, tarfile.TarError) as error:
        raise BundleError(f"Could not read Mermaid distribution and source map: {error}") from error
    asset_digest = hashlib.sha256(asset_bytes).hexdigest()
    source_map_digest = hashlib.sha256(source_map_bytes).hexdigest()
    if asset_digest != metadata["bundled_file_sha256"]:
        raise BundleError("The npm Mermaid distribution is not the exact bundle shipped by Velune")
    if source_map_digest != metadata["source_map_sha256"]:
        raise BundleError("The Mermaid source map changed from its pinned digest")
    try:
        source_map_data = json.loads(source_map_bytes)
        sources = source_map_data["sources"]
        contents = source_map_data["sourcesContent"]
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise BundleError(f"Mermaid source map has no embedded source content: {error}") from error
    if not sources or len(sources) != len(contents) or not all(isinstance(item, str) and item for item in contents):
        raise BundleError("Mermaid source map does not contain complete source for every bundled module")

    source_root_dir = source_root / "mermaid-sources"
    source_root_dir.mkdir(parents=True, exist_ok=True)
    destination = source_root_dir / f"mermaid-{metadata['version']}.min.js.map"
    destination.write_bytes(source_map_bytes)
    manifest = {
        "package": metadata["package"],
        "version": metadata["version"],
        "source_url": url,
        "integrity": integrity,
        "bundled_file": metadata["bundled_file"],
        "bundled_file_sha256": asset_digest,
        "archive_path": destination.relative_to(source_root).as_posix(),
        "source_map_sha256": source_map_digest,
        "embedded_source_count": len(contents),
    }
    manifest_path = source_root / "MERMAID-SOURCES.json"
    manifest_path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return {
        "embedded_source_count": len(contents),
        "source_tree_sha256": tree_digest(source_root_dir),
        "manifest_sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
    }


def vendor_preferred_js_sources(source_root: Path) -> dict[str, object]:
    metadata_path = source_root / "app/mac/Licenses/Mermaid/PREFERRED-SOURCE-SUPPLEMENTS.json"
    try:
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BundleError(f"Could not read preferred JavaScript source metadata: {error}") from error
    archives = metadata.get("archives")
    if metadata.get("schema") != 1 or not isinstance(archives, list) or len(archives) != 28:
        raise BundleError("Preferred JavaScript source metadata must contain the reviewed 28-source set")

    output_root = source_root / "javascript-sources"
    output_root.mkdir(parents=True, exist_ok=True)
    entries: list[dict[str, object]] = []
    for item in archives:
        if not isinstance(item, dict):
            raise BundleError("Malformed preferred JavaScript source entry")
        name = item.get("name")
        version = item.get("version")
        revision = item.get("rev")
        binding = item.get("binding")
        relative = PurePosixPath(str(item.get("path", "")))
        expected_sha = item.get("sha256")
        is_protobuf_inputs = name == "protobuf well-known-type grammar inputs"
        if (
            not isinstance(name, str)
            or not isinstance(version, str)
            or not isinstance(revision, str)
            or not re.fullmatch(r"[0-9a-f]{40}", revision)
            or not isinstance(binding, str)
            or not relative.parts
            or relative.is_absolute()
            or ".." in relative.parts
            or (not is_protobuf_inputs and (not isinstance(expected_sha, str) or not re.fullmatch(r"[0-9a-f]{64}", expected_sha)))
        ):
            raise BundleError(f"Invalid preferred JavaScript source metadata for {name!r}")

        if is_protobuf_inputs:
            if item.get("repo") != "https://github.com/protocolbuffers/protobuf" or not isinstance(item.get("files"), list):
                raise BundleError("Invalid protobuf well-known-type source metadata")
            input_root = output_root / "protobuf-wkt-inputs-36.2"
            file_entries: list[dict[str, str]] = []
            for source in item["files"]:
                if not isinstance(source, dict):
                    raise BundleError("Malformed protobuf grammar input entry")
                source_path = PurePosixPath(str(source.get("path", "")))
                source_url = source.get("url")
                source_sha = source.get("sha256")
                if (
                    source_path.is_absolute()
                    or not source_path.parts
                    or ".." in source_path.parts
                    or not isinstance(source_url, str)
                    or not source_url.startswith(f"https://raw.githubusercontent.com/protocolbuffers/protobuf/{revision}/")
                    or not isinstance(source_sha, str)
                    or not re.fullmatch(r"[0-9a-f]{64}", source_sha)
                ):
                    raise BundleError("Invalid protobuf grammar input URL, path, or digest")
                destination = input_root.joinpath(*source_path.parts)
                destination.parent.mkdir(parents=True, exist_ok=True)
                try:
                    with urlopen(Request(source_url, headers={"User-Agent": "Velune-source-bundle/1"}), timeout=60) as response:
                        payload = response.read()
                except (HTTPError, URLError, TimeoutError) as error:
                    raise BundleError(f"Could not retrieve protobuf grammar input {source_path}: {error}") from error
                if hashlib.sha256(payload).hexdigest() != source_sha:
                    raise BundleError(f"Protobuf grammar input digest changed: {source_path}")
                destination.write_bytes(payload)
                file_entries.append({
                    "path": destination.relative_to(source_root).as_posix(),
                    "source_url": source_url,
                    "sha256": source_sha,
                })
            entry = {
                "name": name,
                "version": version,
                "repository": item["repo"],
                "revision": revision,
                "binding": binding,
                "source_directory": input_root.relative_to(source_root).as_posix(),
                "source_file_count": len(file_entries),
                "source_files": file_entries,
            }
            entries.append(entry)
            continue

        source_url = item.get("url")
        repo = item.get("repo")
        if (
            not isinstance(repo, str)
            or not repo.startswith("https://github.com/")
            or not isinstance(source_url, str)
            or source_url != f"https://codeload.github.com/{repo.removeprefix('https://github.com/')}/tar.gz/{revision}"
        ):
            raise BundleError(f"Unexpected preferred-source repository URL for {name}")
        destination = output_root.joinpath(*relative.parts)
        destination.parent.mkdir(parents=True, exist_ok=True)
        try:
            with urlopen(Request(source_url, headers={"User-Agent": "Velune-source-bundle/1"}), timeout=90) as response:
                payload = response.read()
        except (HTTPError, URLError, TimeoutError) as error:
            raise BundleError(f"Could not retrieve preferred source for {name}@{version}: {error}") from error
        actual_sha = hashlib.sha256(payload).hexdigest()
        if actual_sha != expected_sha:
            raise BundleError(f"Preferred source archive digest changed for {name}@{version}")
        destination.write_bytes(payload)
        entries.append({
            "name": name,
            "version": version,
            "repository": repo,
            "revision": revision,
            "binding": binding,
            "source_url": source_url,
            "archive_path": destination.relative_to(source_root).as_posix(),
            "sha256": actual_sha,
        })

    manifest = {
        "source_set": "Mermaid embedded bundle and runtime-support preferred-source supplements",
        "supplement_count": len(entries),
        "supplements": entries,
        "runtime_npm_source_already_preferred": metadata.get("runtime_npm_already_preferred", []),
        "conditions": metadata.get("conditions", []),
    }
    manifest_path = source_root / "PREFERRED-JS-SOURCES.json"
    manifest_path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return {
        "supplement_count": len(entries),
        "source_tree_sha256": tree_digest(output_root),
        "manifest_sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
    }


def vendor_windows_sources(source_root: Path) -> dict[str, object]:
    """Archive exact NuGet source snapshots and the Velopack Windows helper graph."""
    metadata_path = source_root / "app/windows/Licenses/NUGET-SOURCES.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    unresolved = metadata.get("unresolved_platform_components", [])
    if unresolved:
        names = ", ".join(f"{item['package']} {item['version']}" for item in unresolved)
        raise BundleError(
            "Windows corresponding source is unresolved for bundled platform components: " + names
        )

    lock_path = source_root / metadata["lockfile"]
    if not lock_path.is_file():
        raise BundleError(f"Windows NuGet lock file is missing: {metadata['lockfile']}")
    lock = json.loads(lock_path.read_text(encoding="utf-8"))
    locked = {
        (package.casefold(), entry["resolved"])
        for target in lock.get("dependencies", {}).values()
        for package, entry in target.items()
        if isinstance(entry, dict) and entry.get("resolved")
    }
    manifest_packages = {
        (entry["package"].casefold(), entry["version"])
        for entry in metadata.get("runtime_packages", [])
    }
    unresolved_packages = {
        (entry["package"].casefold(), entry["version"])
        for entry in metadata.get("unresolved_platform_components", [])
    }
    build_only_packages = {
        (entry["package"].casefold(), entry["version"])
        for entry in metadata.get("build_only_packages", [])
    }
    classified = manifest_packages | unresolved_packages | build_only_packages
    if classified != locked:
        missing = sorted(locked - classified)
        stale = sorted(classified - locked)
        raise BundleError(
            f"Windows NuGet source manifest classification differs from packages.lock.json; "
            f"unclassified={missing}, stale={stale}"
        )

    snapshots: dict[tuple[str, str], set[str]] = {}
    for entry in metadata.get("runtime_packages", []):
        repo = entry["repository"]
        revision = entry["revision"]
        source_path = entry.get("source_path", "")
        if not repo.startswith("https://github.com/") or not re.fullmatch(r"[0-9a-f]{40}", revision):
            raise BundleError(f"Windows NuGet source is not a fixed GitHub commit: {entry['package']}")
        if source_path and (PurePosixPath(source_path).is_absolute() or ".." in PurePosixPath(source_path).parts):
            raise BundleError(f"Unsafe source path for Windows package {entry['package']}")
        snapshots.setdefault((repo.removesuffix(".git"), revision), set()).add(source_path)
        license_path = entry.get("repository_license_path")
        if license_path:
            license_relative = PurePosixPath(license_path)
            if license_relative.is_absolute() or ".." in license_relative.parts:
                raise BundleError(f"Unsafe repository license path for Windows package {entry['package']}")
            snapshots[(repo.removesuffix(".git"), revision)].add(license_path)

    velopack = metadata.get("velopack_source", {})
    velopack_repo = velopack.get("repository", "")
    velopack_revision = velopack.get("revision", "")
    if not velopack_repo.startswith("https://github.com/") or not re.fullmatch(r"[0-9a-f]{40}", velopack_revision):
        raise BundleError("Velopack source is not pinned to a full GitHub commit")
    roots = set(velopack.get("source_content_roots", []))
    if not {"Cargo.lock", "Cargo.toml", "LICENSE", "src/"}.issubset(roots):
        raise BundleError("Velopack source roots must include its Rust lock, project, license, and helpers")
    snapshots.setdefault((velopack_repo.removesuffix(".git"), velopack_revision), set()).update(roots)

    destination_root = source_root / "windows-sources"
    snapshot_entries: list[dict[str, object]] = []
    for (repository, revision), paths in sorted(snapshots.items()):
        selected_paths: list[str] = []
        for path in sorted(paths, key=lambda value: (len(PurePosixPath(value.rstrip("/")).parts), value)):
            if any(path.rstrip("/").startswith(parent.rstrip("/") + "/") for parent in selected_paths):
                continue
            selected_paths.append(path)
        slug = repository.removeprefix("https://github.com/").replace("/", "--")
        destination = destination_root / f"{slug}-{revision}"
        with tempfile.TemporaryDirectory(prefix=".windows-source-", dir=destination_root.parent) as temp:
            checkout = Path(temp) / "checkout"
            checkout.mkdir()
            run("git", "init", str(checkout))
            run("git", "-C", str(checkout), "remote", "add", "origin", repository + ".git")
            run("git", "-C", str(checkout), "config", "core.sparseCheckout", "true")
            run("git", "-C", str(checkout), "config", "core.sparseCheckoutCone", "false")
            sparse_rules = []
            for path in selected_paths:
                normalized = path.rstrip("/")
                sparse_rules.extend((f"/{normalized}", f"/{normalized}/"))
            (checkout / ".git/info/sparse-checkout").write_text("\n".join(sparse_rules) + "\n")
            run("git", "-C", str(checkout), "fetch", "--depth=1", "--filter=blob:none", "origin", revision)
            run("git", "-C", str(checkout), "checkout", "--detach", "FETCH_HEAD")
            actual_revision = run("git", "-C", str(checkout), "rev-parse", "HEAD")
            if actual_revision != revision:
                raise BundleError(f"Source checkout resolved to {actual_revision}, expected {revision}")
            if destination.exists():
                raise BundleError(f"Duplicate Windows source destination: {destination}")
            destination.parent.mkdir(parents=True, exist_ok=True)
            for path in selected_paths:
                relative = Path(path.rstrip("/"))
                selected = checkout / relative
                if not selected.exists():
                    raise BundleError(f"Pinned Windows source path is missing: {repository}@{revision}:{path}")
                target = destination / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                if selected.is_dir():
                    shutil.copytree(selected, target, symlinks=True)
                else:
                    shutil.copy2(selected, target)
        snapshot_entries.append({
            "repository": repository,
            "revision": revision,
            "paths": selected_paths,
            "archive_path": destination.relative_to(source_root).as_posix(),
            "sha256": tree_digest(destination),
        })

    rust_snapshot = next(
        (source_root / entry["archive_path"] for entry in snapshot_entries
         if entry["repository"] == velopack_repo and entry["revision"] == velopack_revision),
        None,
    )
    if rust_snapshot is None:
        raise BundleError("Pinned Velopack source snapshot is missing from Windows source archive")
    rust_sources = vendor_velopack_rust_sources(rust_snapshot, source_root / "windows-sources/velopack-rust")
    manifest = {
        "lockfile": metadata["lockfile"],
        "nuget_source_packages": len(metadata.get("runtime_packages", [])),
        "repository_snapshots": snapshot_entries,
        "velopack_rust": rust_sources,
        "unresolved_platform_components": [],
    }
    manifest_path = source_root / "WINDOWS-SOURCES.json"
    manifest_path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return {
        "nuget_source_packages": manifest["nuget_source_packages"],
        "repository_snapshots": len(snapshot_entries),
        "velopack_rust_packages": rust_sources["package_count"],
        "manifest_sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
    }


def vendor_velopack_rust_sources(velopack_root: Path, output_root: Path) -> dict[str, object]:
    """Copy only the locked, non-dev Windows helper dependency closure from Velopack."""
    manifest = velopack_root / "src/bins/Cargo.toml"
    if not manifest.is_file():
        raise BundleError("Velopack source snapshot lacks src/bins/Cargo.toml")
    lock = velopack_root / "Cargo.lock"
    if not lock.is_file():
        raise BundleError("Velopack helper source snapshot lacks its tracked Cargo.lock")
    tree = run(
        "cargo", "tree", "--locked", "--manifest-path", str(manifest), "-p", "velopack_bins",
        "--target", "x86_64-pc-windows-msvc", "--edges", "normal,build", "--prefix", "none",
        "--format", "{p}", cwd=velopack_root,
    )
    names_versions: set[tuple[str, str]] = set()
    for line in tree.splitlines():
        match = re.match(r"^([^\s]+) v([^\s]+)", line.strip())
        if match:
            names_versions.add((match.group(1), match.group(2)))
    if not names_versions:
        raise BundleError("Could not resolve the Velopack Windows helper Cargo dependency closure")

    cargo_lock = lock.read_bytes()
    lock_data = tomllib.loads(cargo_lock.decode("utf-8"))
    lock_records = {
        (package["name"], package["version"]): package
        for package in lock_data.get("package", [])
    }
    copied: list[dict[str, str]] = []
    output_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".velopack-cargo-vendor-") as vendor_temp:
        complete_vendor = Path(vendor_temp) / "vendor"
        run(
            "cargo", "vendor", "--locked", "--versioned-dirs", "--manifest-path", str(manifest),
            str(complete_vendor), cwd=velopack_root,
        )
        for name, version in sorted(names_versions):
            package = lock_records.get((name, version))
            if package is None:
                raise BundleError(f"Velopack Cargo.lock lacks resolved helper package {name} {version}")
            if not package.get("source", ""):
                continue
            if not package["source"].startswith("registry+"):
                raise BundleError(f"Unsupported non-registry Velopack helper dependency: {name} {version}")
            candidates = sorted(complete_vendor.glob(f"{name}-{version}*"))
            if len(candidates) != 1 or not candidates[0].is_dir():
                raise BundleError(f"Expected one vendored Cargo source for Velopack helper dependency {name} {version}")
            checksum_path = candidates[0] / ".cargo-checksum.json"
            if not checksum_path.is_file():
                raise BundleError(f"Vendored Cargo source lacks checksum metadata: {name} {version}")
            checksum = json.loads(checksum_path.read_text(encoding="utf-8"))
            expected_checksum = package.get("checksum")
            if expected_checksum and checksum.get("package") != expected_checksum:
                raise BundleError(f"Cargo registry checksum mismatch for {name} {version}")
            target = output_root / f"{name}-{version}"
            if target.exists():
                raise BundleError(f"Duplicate Velopack helper source directory: {target}")
            shutil.copytree(candidates[0], target)
            copied.append({"name": name, "version": version, "sha256": tree_digest(target)})

    config = velopack_root / ".cargo/config.toml"
    config_text = config.read_text(encoding="utf-8") if config.is_file() else ""
    config_text += (
        "\n[source.crates-io]\nreplace-with = \"velopack-locked-vendor\"\n"
        "[source.velopack-locked-vendor]\ndirectory = \"../velopack-rust\"\n"
    )
    (velopack_root / ".cargo/config.toml").write_text(config_text, encoding="utf-8")
    (output_root.parent / "VELOPPAK-RUST-SOURCES.json").write_text(
        json.dumps({"target": "x86_64-pc-windows-msvc", "lockfile": "Cargo.lock", "packages": copied}, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
    return {
        "target": "x86_64-pc-windows-msvc",
        "package_count": len(copied),
        "source_tree_sha256": tree_digest(output_root),
        "cargo_lock_sha256": hashlib.sha256(cargo_lock).hexdigest(),
    }


def normalized_tar_info(info: tarfile.TarInfo, epoch: int) -> tarfile.TarInfo:
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    info.mtime = epoch
    info.pax_headers = {}
    return info


def write_deterministic_tar_gz(source_root: Path, archive: Path, prefix: str, epoch: int) -> None:
    with archive.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as tar:
            paths = [source_root, *sorted(source_root.rglob("*"), key=lambda item: item.relative_to(source_root).as_posix())]
            for path in paths:
                relative = Path() if path == source_root else path.relative_to(source_root)
                arcname = (PurePosixPath(prefix) / PurePosixPath(relative.as_posix())).as_posix()
                info = normalized_tar_info(tar.gettarinfo(str(path), arcname=arcname), epoch)
                if path.is_file() and not path.is_symlink():
                    with path.open("rb") as stream:
                        tar.addfile(info, stream)
                else:
                    tar.addfile(info)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True, help="Release tag name (without refs/tags/)")
    parser.add_argument("--commit", required=True, help="Full 40-character source commit SHA")
    parser.add_argument("--output-dir", required=True, type=Path, help="Directory for the source archive")
    parser.add_argument(
        "--platforms",
        required=True,
        nargs="+",
        choices=sorted(SUPPORTED_PLATFORMS),
        help="Platforms whose binaries this source archive must cover",
    )
    args = parser.parse_args()

    platforms = sorted(set(args.platforms))
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._+-]*", args.tag):
        raise BundleError("Tag must be a single safe ref name")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", args.commit):
        raise BundleError("--commit must be the full 40-character Git SHA")

    commit = args.commit.lower()
    ref = f"refs/tags/{args.tag}"
    run("git", "check-ref-format", ref)
    tagged_commit = run("git", "rev-parse", f"{ref}^{{commit}}")
    if tagged_commit.lower() != commit:
        raise BundleError(f"Tag {args.tag} resolves to {tagged_commit}, not supplied commit {commit}")
    run("git", "cat-file", "-e", f"{commit}^{{commit}}")
    epoch = int(run("git", "show", "-s", "--format=%ct", commit))

    archive_module = run("git", "show", f"{commit}:{RELAYKIT_GO_MOD.as_posix()}")
    relaykit_version = module_version(archive_module)
    relaykit_match = re.fullmatch(r"v\d+\.\d+\.\d+-0\.\d{14}-([0-9a-f]{12})", relaykit_version)
    if relaykit_match is None:
        raise BundleError(f"Could not parse the RelayKit commit from {relaykit_version}")
    relaykit_commit = relaykit_match.group(1)

    go_output = run("go", "version")
    go_tuple = parse_go_version(go_output)
    if go_tuple < GO_MINIMUM:
        raise BundleError(f"Go 1.25.1 or later is required to vendor RelayKit; found {go_output}")

    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    archive_name = f"velune-{args.tag}-source.tar.gz"
    output = output_dir / archive_name
    root_name = f"velune-{args.tag}"
    environment = os.environ.copy()
    environment["GOWORK"] = "off"
    environment["GOTOOLCHAIN"] = "local"

    with tempfile.TemporaryDirectory(prefix=".velune-source-bundle-", dir=output_dir) as temporary:
        temporary_root = Path(temporary)
        git_archive = temporary_root / "source.tar"
        run("git", "archive", "--format=tar", f"--prefix={root_name}/", "--output", str(git_archive), commit)
        source_root = safe_extract_git_archive(git_archive, temporary_root, root_name)

        windows_sources = vendor_windows_sources(source_root) if "windows" in platforms else None

        cargo_vendor_sha256 = vendor_cargo_sources(source_root)
        cargo_output = run("cargo", "--version", cwd=source_root)
        android_sources = vendor_android_sources(source_root)
        swift_sources = vendor_swift_sources(source_root)
        npm_sources = vendor_npm_sources(source_root)
        mermaid_sources = vendor_mermaid_sources(source_root)
        preferred_js_sources = vendor_preferred_js_sources(source_root)
        module_dir = source_root / RELAYKIT_GO_MOD.parent
        run("go", "mod", "vendor", cwd=module_dir, env=environment)
        vendor_modules = module_dir / "vendor" / "modules.txt"
        if not vendor_modules.is_file() or not any(
            line.strip() == f"# {RELAYKIT_MODULE} {relaykit_version}"
            for line in vendor_modules.read_text(encoding="utf-8").splitlines()
        ):
            raise BundleError("Go vendoring did not include the exact RelayKit module version")

        manifest = {
            "source_tag": args.tag,
            "source_commit": commit,
            "corresponding_platforms": platforms,
            "relaykit_module": RELAYKIT_MODULE,
            "relaykit_version": relaykit_version,
            "relaykit_commit_prefix": relaykit_commit,
            "cargo_version": cargo_output,
            "android_source_artifacts": android_sources,
            "swift_sources": swift_sources,
            "npm_sources": npm_sources,
            "mermaid_sources": mermaid_sources,
            "preferred_js_sources": preferred_js_sources,
            "windows_sources": windows_sources,
            "go_version": go_output,
            "cargo_vendor_sha256": cargo_vendor_sha256,
            "source_tree_sha256": source_tree_digest(source_root),
            "vendored_source_sha256": tree_digest(module_dir / "vendor"),
        }
        (source_root / "SOURCE-ARCHIVE.json").write_text(
            json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
        )

        temporary_output = temporary_root / archive_name
        write_deterministic_tar_gz(source_root, temporary_output, root_name, epoch)
        os.replace(temporary_output, output)

    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    print(f"Created {output} ({digest})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BundleError as error:
        print(f"source bundle: {error}", file=sys.stderr)
        raise SystemExit(2)
