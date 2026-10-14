#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export NO_DNA=1

# Uses the already installed toolchain and cached dependencies; no cluster access.
for version in v1 v2; do
  cargo build-sbf --tools-version v1.57 --offline --skip-tools-install \
    --manifest-path "programs/dg-escrow-$version/Cargo.toml" \
    --sbf-out-dir "target/deploy/escrow-$version"
done
cargo build-sbf --tools-version v1.57 --offline --skip-tools-install \
  --manifest-path programs/dg-escrow-v2/Cargo.toml \
  --features r9-broken-amount --sbf-out-dir target/deploy/escrow-broken

mkdir -p fixtures/escrow/idl
for version in v1 v2; do
  "${ANCHOR_BIN:-anchor}" idl build -p "dg_escrow_$version" \
    -o "fixtures/escrow/idl/$version.json" -- --offline
done
