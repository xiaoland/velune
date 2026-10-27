"""Make bounded, metadata-only diagnostic cases from manual-telemetry's actual wire capture.

Usage: python3 agent-cases.py <acceptance basename without .json> <output directory>
The injection ledger belongs to the evaluator and is deliberately excluded.
"""
import base64
import json
from pathlib import Path
import sys


def attributes(record):
    return {item["key"]: next(iter(item.get("value", {}).values()), None) for item in record.get("attributes", [])}


def main():
    base, output = Path(sys.argv[1]), Path(sys.argv[2])
    boundaries = json.loads(base.with_suffix(".boundaries.json").read_text())
    wire = json.loads(base.with_suffix(".wire.json").read_text())
    records = []
    for batches in wire:
        for batch in batches:
            for resources in batch["payload"].values():
                for resource in resources:
                    instance = attributes(resource.get("resource", {})).get("service.instance.id")
                    for scope in resource.get("scopeSpans", []):
                        for span in scope.get("spans", []):
                            records.append({"type": "span", "instance": instance, "scope": scope["scope"]["name"],
                                            "traceId": base64.b64decode(span["traceId"]).hex(), "spanId": base64.b64decode(span["spanId"]).hex(),
                                            "parentSpanId": base64.b64decode(span.get("parentSpanId", "")).hex(), "startUnixNs": int(span["startTimeUnixNano"]),
                                            "endUnixNs": int(span["endTimeUnixNano"]), "attributes": attributes(span)})
                    for scope in resource.get("scopeLogs", []):
                        for log in scope.get("logRecords", []):
                            records.append({"type": "log", "instance": instance, "scope": scope["scope"]["name"],
                                            "traceId": base64.b64decode(log.get("traceId", "")).hex(), "timeUnixNs": int(log.get("timeUnixNano") or log["observedTimeUnixNano"]),
                                            "attributes": attributes(log), "body": log.get("body")})
    distributed = boundaries["distributed"]
    cases = [("A", [distributed["preparationFailedParent"]]), ("B", distributed["parents"]), ("C", [boundaries["gateway"]["parents"]["retry"]])]
    output.mkdir(parents=True, exist_ok=True)
    for name, parents in cases:
        traces = {parent.split("-")[1] for parent in parents}
        selected = [record for record in records if record["traceId"] in traces]
        start = min(record.get("startUnixNs", record.get("timeUnixNs")) for record in selected)
        end = max(record.get("endUnixNs", record.get("timeUnixNs")) for record in selected)
        state = None
        if name == "B":
            end = distributed["disconnectedAtUnixMs"] * 1_000_000
            selected = [record for record in selected if record.get("endUnixNs", record.get("timeUnixNs")) <= end]
            state = {"instance": distributed["hostInstance"], "sampleRatio": 1, "activeTurns": 2,
                     "ownerAvailable": True, "observedAtQueryEnd": True, "controllerConnection": "closed",
                     "source": "actual FFI snapshot assertion at the disconnect boundary"}
        assert len(selected) <= 100
        document = {"case": name, "query": {"startUnixNs": start, "endUnixNs": end, "recordLimit": 100, "truncated": False,
                    "sampleRatio": 1, "source": "actual OTLP receiver capture; synthetic data", "externalHarnessInternalsVisible": False},
                    "instances": sorted({record["instance"] for record in selected}), "state": state, "records": selected}
        (output / (name + ".json")).write_text(json.dumps(document, indent=2) + "\n")
    print("3 bounded diagnostic cases prepared")


if __name__ == "__main__":
    main()
