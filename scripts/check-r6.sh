#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Original Vault binaries (including faulty V2) and IDLs are acceptance prerequisites.
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-r6-fixture-v1/Cargo.toml --sbf-out-dir target/deploy/r6-v1
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-r6-fixture-v2/Cargo.toml --sbf-out-dir target/deploy/r6-v2
cargo test -p dg-runtime --test runtime_r6
cargo test -p dg-runtime --test vault_r6_acceptance
cargo test --workspace
cargo clippy -p dg-runtime -p dg-config -p dg-state -p dg-cli -p dg-r6-fixture-v1 -p dg-r6-fixture-v2 --all-targets --no-deps -- -D warnings
cargo fmt --all --check
cargo run -p dg-cli -- runtime migrate examples/vault-runtime --output report/migration-execution.json
