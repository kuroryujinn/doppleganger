#!/usr/bin/env bash
set -euo pipefail
task_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$task_root"
mkdir -p "$task_root/report"
task_evidence="$(mktemp -d "$task_root/report/backend-foundation.XXXXXX")"
exec > >(tee "$task_evidence/acceptance.log") 2>&1
cargo fmt --all -- --check
cargo clippy -p dg-service --all-targets --locked -- -D warnings
cargo test -p dg-service --locked
DG_QUALIFIED_ENGINE="${DG_QUALIFIED_ENGINE:-$task_root/target/release/doppelganger}" \
DG_PREPARED_EXAMPLES="${DG_PREPARED_EXAMPLES:-$task_root/report/releases/doppelganger-0.1.0-linux-x86_64/examples}" \
  cargo test -p dg-service --locked --test engine_contract -- --ignored
printf 'Backend foundation evidence: %s\n' "$task_evidence"
