#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Use the same SBF toolchain as the existing demo. Vault artifacts are prerequisites
# for the separately invoked acceptance test, not for isolated R5 tests.
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-r5-fixture/Cargo.toml --sbf-out-dir target/deploy/r5
cargo test -p dg-state -p dg-config
cargo test -p dg-runtime --test runtime_r5
cargo test -p dg-runtime --test vault_equivalence
cargo test --workspace
cargo clippy -p dg-state -p dg-config -p dg-runtime -p dg-cli -p dg-r5-fixture --all-targets --no-deps -- -D warnings
cargo run -p dg-cli -- runtime build examples/vault-runtime --output report/v1-world.json
