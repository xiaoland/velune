#!/usr/bin/env python3
"""Package one already-built Velune release artifact and its provenance."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import xml.etree.ElementTree as ET
import zipfile
from pathlib import PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from version import read_build_number, read_version  # noqa: E402


def sha256(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def digest(path: Path, algorithm: str) -> str:
    value = hashlib.new(algorithm)
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def source_platforms(archive: Path, tag: str, commit: str) -> list[str]:
    with tarfile.open(archive, "r:gz") as source:
        manifests = [
            member for member in source.getmembers()
            if member.isfile()
            and PurePosixPath(member.name).name == "SOURCE-ARCHIVE.json"
            and len(PurePosixPath(member.name).parts) == 2
        ]
        if len(manifests) != 1:
            raise ValueError("source archive must contain exactly one root SOURCE-ARCHIVE.json")
        stream = source.extractfile(manifests[0])
        if stream is None:
            raise ValueError("cannot read source archive identity manifest")
        metadata = json.loads(stream.read())
    if metadata.get("source_tag") != tag or metadata.get("source_commit") != commit:
        raise ValueError("source archive manifest tag/commit does not match the release identity")
    platforms = metadata.get("corresponding_platforms")
    allowed = {"macos", "windows", "android", "ios"}
    if (not isinstance(platforms, list) or not platforms
            or any(not isinstance(item, str) or item not in allowed for item in platforms)
            or len(set(platforms)) != len(platforms)):
        raise ValueError("source archive manifest has an invalid corresponding_platforms list")
    return sorted(platforms)


def validate_macos_bundle(bundle: Path, version: str, core: str, build_number: int, commit: str, architecture: str) -> None:
    from macos_identity import validate_context

    info_path = bundle / "Contents/Info.plist"
    manifest_path = bundle / "Contents/Resources/build-manifest.json"
    info = plistlib.loads(info_path.read_bytes())
    context = validate_context(info, "daily")
    if info.get("CFBundleShortVersionString") != core:
        raise ValueError("macOS CFBundleShortVersionString does not match version.txt numeric core")
    if info.get("VeluneDisplayVersion") != version:
        raise ValueError("macOS VeluneDisplayVersion does not match version.txt SemVer")
    if str(info.get("CFBundleVersion")) != str(build_number):
        raise ValueError("macOS CFBundleVersion does not match the release build number")
    manifest = json.loads(manifest_path.read_text())
    if (manifest.get("ui_version") != version or manifest.get("source_commit") != commit
            or manifest.get("dirty") is not False or manifest.get("profile") != "release"
            or manifest.get("purpose") != "daily" or manifest.get("architecture") != architecture
            or manifest.get("execution_context") != context):
        raise ValueError("macOS build manifest does not match the release version, source, architecture, or daily purpose")
    executable = bundle / "Contents/MacOS" / context["executable"]
    if not executable.is_file() or executable.is_symlink():
        raise ValueError("macOS daily executable is missing or unsafe")
    archs = subprocess.check_output(["lipo", "-archs", str(executable)], text=True).split()
    if set(archs) != {architecture}:
        raise ValueError(f"macOS binary architectures {sorted(set(archs))} do not match {architecture}")


def android_aapt() -> Path:
    explicit = os.environ.get("VELUNE_AAPT")
    if explicit:
        path = Path(explicit)
        if path.is_file():
            return path
        raise ValueError(f"VELUNE_AAPT does not point to an aapt executable: {path}")
    on_path = shutil.which("aapt")
    if on_path:
        return Path(on_path)
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if sdk:
        candidates = [Path(sdk) / "build-tools" / version / "aapt" for version in os.listdir(Path(sdk) / "build-tools")]
        candidates = [path for path in candidates if path.is_file()]
        if candidates:
            def version_key(path: Path) -> tuple[int, ...]:
                return tuple(int(part) if part.isdigit() else 0 for part in path.parent.name.split("."))
            return max(candidates, key=version_key)
    raise ValueError("Android packaging requires the Android SDK build-tools aapt command")


def validate_android_apk(apk: Path, version: str, build_number: int) -> None:
    output = subprocess.check_output([str(android_aapt()), "dump", "badging", str(apk)], text=True)
    match = re.search(r"(?m)^package: name='([^']+)' versionCode='([^']+)' versionName='([^']*)'", output)
    if match is None:
        raise ValueError("aapt could not read package name/versionCode/versionName from the release APK")
    package_name, code, name = match.groups()
    if package_name != "app.velune.android":
        raise ValueError(f"Android applicationId {package_name!r} does not match app.velune.android")
    if name != version or code != str(build_number):
        raise ValueError("Android APK versionName/versionCode do not match version.txt and the release build number")
    with zipfile.ZipFile(apk) as package:
        abis = {
            PurePosixPath(name).parts[1]
            for name in package.namelist()
            if name.startswith("lib/") and name.endswith(".so") and len(PurePosixPath(name).parts) >= 3
        }
    if abis != {"arm64-v8a"}:
        raise ValueError(f"Android APK native ABIs {sorted(abis)} do not match the arm64 release target")


def windows_product_version(assembly: Path) -> str:
    shell = shutil.which("pwsh") or shutil.which("powershell.exe") or shutil.which("powershell")
    if not shell:
        raise ValueError("Windows packaging requires PowerShell FileVersionInfo to read the built assembly")
    env = {**os.environ, "VELUNE_ASSEMBLY_PATH": str(assembly)}
    script = (
        "$v = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($env:VELUNE_ASSEMBLY_PATH); "
        "if (-not $v.ProductVersion) { exit 2 }; [Console]::Out.Write($v.ProductVersion)"
    )
    return subprocess.check_output([shell, "-NoProfile", "-NonInteractive", "-Command", script], env=env, text=True).strip()


def validate_velopack_feed(feed: Path, package: Path, version: str) -> None:
    metadata = json.loads(feed.read_text())
    assets = metadata.get("Assets")
    if not isinstance(assets, list):
        raise ValueError("Velopack channel feed must contain an Assets list")
    matching = [
        item for item in assets
        if isinstance(item, dict) and item.get("PackageId") == "Velune"
        and item.get("Version") == version and item.get("Type") == "Full"
    ]
    if len(matching) != 1:
        raise ValueError("Velopack feed must contain exactly one full package for this version")
    item = matching[0]
    if (item.get("FileName") != package.name or item.get("Size") != package.stat().st_size
            or item.get("SHA1", "").lower() != digest(package, "sha1")):
        raise ValueError("Velopack feed package filename, size, or SHA-1 does not match the full package")


def zip_member_sha256(package: zipfile.ZipFile, member: zipfile.ZipInfo) -> str:
    value = hashlib.sha256()
    with package.open(member) as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def unique_package_member(package: zipfile.ZipFile, basename: str) -> zipfile.ZipInfo:
    matches = [
        member for member in package.infolist()
        if not member.is_dir() and PurePosixPath(member.filename).name.casefold() == basename.casefold()
    ]
    if len(matches) != 1:
        raise ValueError(f"Velopack full package must contain exactly one {basename}")
    return matches[0]


def validate_velopack_package(package_path: Path, publish_dir: Path, version: str) -> list[dict[str, str]]:
    """Bind the Velopack payload and nuspec identity to the already-validated publish tree."""
    with zipfile.ZipFile(package_path) as package:
        nuspecs = [
            member for member in package.infolist()
            if not member.is_dir() and PurePosixPath(member.filename).suffix.casefold() == ".nuspec"
        ]
        if len(nuspecs) != 1:
            raise ValueError("Velopack full package must contain exactly one nuspec")
        root = ET.fromstring(package.read(nuspecs[0]))
        metadata_nodes = [node for node in root if node.tag.rsplit("}", 1)[-1].casefold() == "metadata"]
        if len(metadata_nodes) != 1:
            raise ValueError("Velopack nuspec must contain exactly one metadata element")
        metadata = metadata_nodes[0]

        def field(name: str) -> str:
            values = [
                node.text.strip() for node in metadata
                if node.tag.rsplit("}", 1)[-1].casefold() == name.casefold() and node.text and node.text.strip()
            ]
            if len(values) != 1:
                raise ValueError(f"Velopack nuspec must contain exactly one {name}")
            return values[0]

        if field("id").casefold() != "velune" or field("version") != version:
            raise ValueError("Velopack nuspec ID/version do not match Velune and version.txt")

        verified = []
        for basename in ("Velune.dll", "velune_bindings.dll", "velune_relaykit.dll"):
            source = publish_dir / basename
            if not source.is_file():
                raise ValueError(f"validated Windows publish directory is missing {basename}")
            member = unique_package_member(package, basename)
            source_hash = sha256(source)
            package_hash = zip_member_sha256(package, member)
            if package_hash != source_hash:
                raise ValueError(f"Velopack package {basename} differs from the validated Windows publish tree")
            verified.append({"file": basename, "sha256": source_hash})
        return verified


def package(args: argparse.Namespace) -> Path:
    source = Path(args.input).resolve()
    output_dir = Path(args.output_dir).resolve()
    covered_platforms = None
    if not source.exists():
        raise ValueError(f"release input does not exist: {source}")
    version, _ = read_version()
    if args.version != version:
        raise ValueError(f"artifact version {args.version} does not match version.txt {version}")
    _, version_core = read_version()
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", args.tag):
        raise ValueError("tag must be a v-prefixed SemVer release tag")
    if not re.fullmatch(r"[0-9a-f]{40,64}", args.commit):
        raise ValueError("commit must be a full lowercase Git object ID")
    if not re.fullmatch(r"[1-9][0-9]*", args.build_number):
        raise ValueError("build number must be a positive decimal integer")

    output_dir.mkdir(parents=True, exist_ok=True)
    build_number = read_build_number(args.build_number)
    binary_version = None
    verified_package_payloads = None
    if args.kind == "source":
        if not source.is_file() or source.suffixes[-2:] != [".tar", ".gz"]:
            raise ValueError("source input must be the verified .tar.gz bundle")
        covered_platforms = source_platforms(source, args.tag, args.commit)
        suffix = "source.tar.gz"
        signing = "source archive; Cargo.lock Rust sources and RelayKit Go module sources are vendored"
    elif args.kind == "macos":
        if args.arch not in ("arm64", "x86_64") or not source.is_dir() or source.suffix != ".app":
            raise ValueError("macOS input must be a .app bundle for arm64 or x86_64")
        validate_macos_bundle(source, version, version_core, build_number, args.commit, args.arch)
        binary_version = version
        suffix = f"macOS-{args.arch}.zip"
        signing = "ad-hoc; not notarized"
    elif args.kind == "windows":
        if not args.velopack_dir:
            raise ValueError("Windows packaging requires the output directory from vpk pack")
        if args.arch != "x64" or not source.is_dir():
            raise ValueError("Windows input must be the self-contained x64 publish directory")
        assembly = source / "Velune.dll"
        if not assembly.is_file() or not (source / "Velune.exe").is_file():
            raise ValueError("Windows publish directory must contain the Velune.dll assembly and x64 apphost")
        binary_version = windows_product_version(assembly)
        if binary_version != version:
            raise ValueError(f"Windows assembly ProductVersion {binary_version!r} does not match {version}")
        vpk_dir = Path(args.velopack_dir).resolve()
        channel = args.channel
        if channel != "win-x64-beta":
            raise ValueError("Windows release channel must remain win-x64-beta")
        package_name = f"Velune-{version}-{channel}-full.nupkg"
        feed_name = f"releases.{channel}.json"
        setup_name = f"Velune-{channel}-Setup.exe"
        package_source = vpk_dir / package_name
        feed_source = vpk_dir / feed_name
        setup_source = vpk_dir / setup_name
        for required in (package_source, feed_source, setup_source):
            if not required.is_file() or required.stat().st_size == 0:
                raise ValueError(f"Velopack output is missing or empty: {required.name}")
        validate_velopack_feed(feed_source, package_source, version)
        verified_package_payloads = validate_velopack_package(package_source, source, version)
        suffix = "windows-x64-setup.exe"
        signing = "unsigned; no Authenticode signature"
    elif args.kind == "android":
        if args.arch != "arm64" or not source.is_file() or source.suffix.lower() != ".apk":
            raise ValueError("Android input must be a signed arm64 APK")
        if not args.certificate_sha256 or not re.fullmatch(r"[0-9a-f]{64}", args.certificate_sha256):
            raise ValueError("Android release requires the verified signing certificate SHA-256")
        validate_android_apk(source, version, build_number)
        binary_version = version
        suffix = "android-arm64.apk"
        signing = "APK v1/v2/v3 signature; certificate SHA-256 " + args.certificate_sha256

    prefix = "velune" if args.kind == "source" else "Velune"
    name = f"Velune-{args.channel}-Setup.exe" if args.kind == "windows" else f"{prefix}-{args.tag}-{suffix}"
    artifact = output_dir / name
    if args.kind == "macos":
        subprocess.run(
            ["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(source), str(artifact)],
            check=True,
        )
    elif args.kind == "windows":
        assets = []
        for role, vpk_source, target_name in (
            ("installer", setup_source, name),
            ("full_package", package_source, package_name),
            ("update_feed", feed_source, feed_name),
        ):
            target = output_dir / target_name
            shutil.copyfile(vpk_source, target)
            assets.append({"role": role, "name": target.name, "sha256": sha256(target)})
    else:
        shutil.copyfile(source, artifact)

    metadata = {
        "tag": args.tag,
        "version": args.version,
        "binary_version": binary_version,
        "source_commit": args.commit,
        "build_number": build_number,
        "platform": args.kind,
        "architecture": args.arch,
        "artifact": name,
        "sha256": sha256(artifact),
        "signing": signing,
        "notarized": False if args.kind == "macos" else None,
        "authenticode_signed": False if args.kind == "windows" else None,
        "certificate_sha256": args.certificate_sha256 if args.kind == "android" else None,
        "corresponding_platforms": covered_platforms,
    }
    if args.kind == "windows":
        metadata["channel"] = args.channel
        metadata["assets"] = assets
        metadata["verified_package_payloads"] = verified_package_payloads
    (output_dir / f"{name}.build.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(artifact)
    return artifact


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("macos", "windows", "android", "source"))
    parser.add_argument("--input", required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--build-number", required=True)
    parser.add_argument("--arch", required=True)
    parser.add_argument("--certificate-sha256")
    parser.add_argument("--velopack-dir")
    parser.add_argument("--channel")
    args = parser.parse_args()
    try:
        package(args)
    except (OSError, ValueError, subprocess.CalledProcessError, zipfile.BadZipFile, ET.ParseError) as error:
        print(f"release package: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
