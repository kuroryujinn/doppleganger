#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

v1=target/deploy/v1/dg_vault_v1.so
good=target/deploy/v2/dg_vault_v2.so
bad=target/deploy/bad/dg_vault_v2.so
v1_idl=target/idl/v1.json
v2_idl=target/idl/v2.json

for artifact in "$v1" "$good" "$bad" "$v1_idl" "$v2_idl"; do
    if [[ ! -f "$artifact" ]]; then
        printf 'Missing %s. Build the programs and IDLs using README.md first.\n' "$artifact" >&2
        exit 2
    fi
done

cargo run -q -p dg-cli -- demo "$v1" "$good" "$v1_idl" "$v2_idl" report/good.json
if cargo run -q -p dg-cli -- demo "$v1" "$bad" "$v1_idl" "$v2_idl" report/bad.json; then
    printf 'Faulty V2 unexpectedly passed.\n' >&2
    exit 1
fi

python3 -c '
import json
from pathlib import Path

good = json.loads(Path("report/good.json").read_text())
bad = json.loads(Path("report/bad.json").read_text())
assert all(check["passed"] for check in good["checks"])
failures = {check["name"] for check in bad["checks"] if not check["passed"]}
assert failures == {"sol_backing_preserved", "post_migration_sol_backing"}, failures
print("MVP smoke test passed: corrected migration passes; faulty migration loses SOL backing.")
'

cargo run -q -p dg-cli -- project add examples/vault-project.toml
cargo run -q -p dg-cli -- project add examples/vault-schema-only.toml
cargo run -q -p dg-cli -- project check vault-demo
cargo run -q -p dg-cli -- project check vault-schema-only

python3 -c '
import json
from pathlib import Path

runtime = json.loads(Path("report/vault-demo.json").read_text())
static = json.loads(Path("report/vault-schema-only.json").read_text())
assert runtime["coverage"] == "runtime passed"
assert all(check["passed"] for check in runtime["runtime"]["checks"])
assert static["coverage"] == "schema only; migration not executed"
assert static["runtime"] is None
assert static["accounts"][0]["pda_seeds_changed"] is None
print("Project registration and both coverage modes passed.")
'
