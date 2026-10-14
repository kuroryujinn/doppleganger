#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p report/mvp-qualification
"${DG_AUDIT_BIN:-cargo-audit}" audit --json > report/mvp-qualification/audit.json
sha256sum -c tests/compatibility/SHA256SUMS
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --no-deps --offline -- -D warnings
cargo test --workspace --offline
cargo build -p dg-cli --offline

mkdir -p report
run_root=$(mktemp -d "$PWD/report/production-mvp.XXXXXX")
target/debug/doppelganger migration test examples/vault-runtime --ci --output "$run_root/vault"
target/debug/doppelganger migration test examples/escrow-runtime --ci --output "$run_root/escrow"
target/debug/doppelganger test examples/record-runtime --ci --output "$run_root/record"
set +e
target/debug/doppelganger migration test examples/escrow-runtime-broken --ci \
    --policy examples/escrow-runtime/doppelganger-policy.toml --output "$run_root/broken"
broken_exit=$?
set -e
if [[ "$broken_exit" != 1 ]]; then
    printf 'Expected broken migration to return policy violation 1, got %s\n' "$broken_exit" >&2
    exit 1
fi
target/debug/doppelganger regression capture --from-report "$run_root/broken/report.json" --store "$run_root/regression"
target/debug/doppelganger regression replay "$run_root/regression" --output "$run_root/reproduced"
python3 - "$run_root" <<'PY'
import json
import sys
from pathlib import Path

reports = Path(sys.argv[1])
for name in ("vault", "escrow", "record"):
    report = json.loads((reports / name / "report.json").read_text())
    assert report["result"] == "PASS" and report["policy"]["blocking"] == 0
broken = json.loads((reports / "broken" / "report.json").read_text())
assert broken["execution"]["transaction"]["success"] is True
assert broken["policy"]["blocking"] == 1
assert broken["preservation"]["failed"] == 1
assert broken["preservation"]["findings"][0]["id"] == "PRESERVE-amount-retained"
assert broken["relationships"]["failed"] == broken["relationships"]["errors"] == 0
assert broken["lifecycle"]["failed"] == broken["lifecycle"]["errors"] == 0
PY
printf 'Production CLI acceptance passed. Reports: %s\n' "$run_root"
