#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
try:
    import jsonschema  # noqa: F401
except Exception as exc:
    raise SystemExit(f"python jsonschema package is required: {exc}")
PY

TMP_DIR="$(mktemp -d)"
TMP_STORE="$TMP_DIR/dev.sqlite"
TMP_STREAM_STORE="$TMP_DIR/dev-stream.sqlite"
TMP_ACCEPT_STORE="$TMP_DIR/accept.sqlite"
TMP_HUMAN_ACCEPT_STORE="$TMP_DIR/human-accept.sqlite"
trap 'rm -rf "$TMP_DIR"' EXIT

cargo run --quiet -p whipplescript -- --json check examples/provider-language-e2e.whip \
  > "$TMP_DIR/check.json"
cargo run --quiet -p whipplescript -- --json compile examples/provider-language-e2e.whip \
  > "$TMP_DIR/compile.json"
cargo run --quiet -p whipplescript -- --store "$TMP_STORE" --json dev \
  examples/provider-language-e2e.whip --provider fixture --until idle \
  > "$TMP_DIR/dev.json"
DEV_INSTANCE_ID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["instance_id"])' "$TMP_DIR/dev.json")"
cargo run --quiet -p whipplescript -- --store "$TMP_STORE" --json trace \
  "$DEV_INSTANCE_ID" --check \
  > "$TMP_DIR/trace.json"
cargo run --quiet -p whipplescript -- --store "$TMP_STREAM_STORE" dev \
  examples/provider-language-e2e.whip --provider fixture --until idle --stream ndjson \
  > "$TMP_DIR/dev-stream.ndjson"
cargo run --quiet -p whipplescript -- --store "$TMP_ACCEPT_STORE" --json accept \
  examples/provider-language-e2e.accept.json \
  > "$TMP_DIR/acceptance.json"
cargo run --quiet -p whipplescript -- --store "$TMP_HUMAN_ACCEPT_STORE" --json accept \
  examples/human-review.accept.json \
  > "$TMP_DIR/human-acceptance.json"

python3 - "$TMP_DIR" <<'PY'
import json
import sys
from pathlib import Path
from jsonschema import Draft202012Validator

tmp_dir = Path(sys.argv[1])
pairs = [
    ("spec/report-schemas/check_report_v0.schema.json", tmp_dir / "check.json"),
    ("spec/report-schemas/compile_report_v0.schema.json", tmp_dir / "compile.json"),
    ("spec/report-schemas/dev_report_v0.schema.json", tmp_dir / "dev.json"),
    ("spec/report-schemas/local_trace_v0.schema.json", tmp_dir / "trace.json"),
    ("spec/report-schemas/acceptance_fixture_v0.schema.json", Path("examples/provider-language-e2e.accept.json")),
    ("spec/report-schemas/acceptance_report_v0.schema.json", tmp_dir / "acceptance.json"),
    ("spec/report-schemas/acceptance_fixture_v0.schema.json", Path("examples/human-review.accept.json")),
    ("spec/report-schemas/acceptance_report_v0.schema.json", tmp_dir / "human-acceptance.json"),
]

for schema_path, report_path in pairs:
    schema = json.loads(Path(schema_path).read_text())
    report = json.loads(report_path.read_text())
    Draft202012Validator(schema).validate(report)
    print(f"validated {report_path.name} against {Path(schema_path).name}")

stream_schema = json.loads(Path("spec/report-schemas/dev_stream_v0.schema.json").read_text())
dev_schema = json.loads(Path("spec/report-schemas/dev_report_v0.schema.json").read_text())
stream_events = [
    json.loads(line)
    for line in (tmp_dir / "dev-stream.ndjson").read_text().splitlines()
    if line.strip()
]
if not stream_events:
    raise SystemExit("dev-stream.ndjson did not contain events")
for index, event in enumerate(stream_events):
    Draft202012Validator(stream_schema).validate(event)
    if event.get("sequence") != index:
        raise SystemExit(f"stream sequence mismatch at line {index}")
final = stream_events[-1]
if final.get("event") != "dev.report":
    raise SystemExit("final stream event was not dev.report")
Draft202012Validator(dev_schema).validate(final["data"])
print("validated dev-stream.ndjson against dev_stream_v0.schema.json")
PY
