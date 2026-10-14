#!/usr/bin/env bash
set -euo pipefail
task_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$task_root"
mkdir -p "$task_root/report"
task_evidence="$(mktemp -d "$task_root/report/backend-worker.XXXXXX")"
exec > >(tee "$task_evidence/acceptance.log") 2>&1
cargo fmt --all -- --check
cargo clippy -p dg-service --all-targets --locked -- -D warnings
cargo test -p dg-service --locked
DG_QUALIFIED_ENGINE="${DG_QUALIFIED_ENGINE:-$task_root/target/release/doppelganger}" \
DG_PREPARED_EXAMPLES="${DG_PREPARED_EXAMPLES:-$task_root/report/releases/doppelganger-0.1.0-linux-x86_64/examples}" \
  cargo test -p dg-service --locked --test engine_contract -- --ignored
if [[ "${1:-}" == "--container" ]]; then
  : "${DG_WORKER_CONFIG:?Set an actual rootless worker config}"
  DG_QUALIFIED_ENGINE="${DG_QUALIFIED_ENGINE:-$task_root/target/release/doppelganger}" \
  DG_PREPARED_EXAMPLES="${DG_PREPARED_EXAMPLES:-$task_root/report/releases/doppelganger-0.1.0-linux-x86_64/examples}" \
  DG_BROKEN_ESCROW="${DG_BROKEN_ESCROW:-$task_root/target/deploy/escrow-broken/dg_escrow_v2.so}" \
    cargo test -p dg-service --locked --test container_contract -- --ignored
  DG_WORKER_CONFIG="$DG_WORKER_CONFIG" \
  DG_PREPARED_EXAMPLES="${DG_PREPARED_EXAMPLES:-$task_root/report/releases/doppelganger-0.1.0-linux-x86_64/examples}" \
    cargo test -p dg-service --locked --test container_faults -- --ignored --nocapture
  printf 'Live container fixture and worker fault acceptance passed. Evidence: %s\n' "$task_evidence"
elif [[ -n "${1:-}" ]]; then
  printf 'Usage: scripts/check-backend-worker.sh [--container]\n' >&2
  exit 2
else
  printf 'Foundation, worker and API tests passed; live container qualification NOT RUN. Evidence: %s\n' "$task_evidence"
fi
