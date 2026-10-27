"""Reuse SwiftPM's resolved telemetry modules in standalone Mac acceptance programs."""
import json
import shlex
from pathlib import Path


def add_app_sources(command, repository, directory, store):
    """Compile the current AppStore graph without maintaining a stale subset of UI types."""
    for source in sorted((Path(repository) / "app/mac").glob("*.swift")):
        if source.name in ("ClientTelemetry.swift", "TelemetryExporters.swift"):
            continue  # add_telemetry supplies these alongside the SDK objects.
        if source.name == "Store.swift":
            command.append(str(store))
        elif source.name == "main.swift":
            declarations = Path(directory) / "AppDeclarations.swift"
            text = source.read_text()
            assert text.count("@main") == 1
            declarations.write_text(text.replace("@main", ""))
            command.append(str(declarations))
        else:
            command.append(str(source))


def add_telemetry(command, build, repository):
    build = Path(build).resolve()
    description = json.loads((build / "description.json").read_text())
    arguments = next(value["otherArguments"] for value in description["swiftCommands"].values() if value["moduleName"] == "VeluneMac")
    for index, argument in enumerate(arguments[:-1]):
        if argument == "-Xcc":
            command.extend((argument, arguments[index + 1]))
    existing = set(command)
    objects = [shlex.split(line)[0] for line in (build / "VeluneMac.product/Objects.LinkFileList").read_text().splitlines()]
    command.extend(path for path in objects if "/VeluneMac.build/" not in path and path not in existing)
    command.extend(str(repository / "app/mac" / name) for name in ("ClientTelemetry.swift", "TelemetryExporters.swift"))
