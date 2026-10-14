#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export NO_DNA=1

# A provisioned runner supplies platform-tools v1.57 and Anchor 1.2.0.
build() {
    local package="$1" destination="$2"
    shift 2
    cargo build-sbf --tools-version v1.57 --offline --skip-tools-install \
        --manifest-path "programs/$package/Cargo.toml" \
        --sbf-out-dir "target/deploy/$destination" "$@"
}
build dg-vault-v1 v1
build dg-vault-v2 v2
build dg-vault-v2 bad --features skip-backing-top-up
build dg-vault-v2 r7-broken --features r7-broken-balance
build dg-vault-v2 r8-broken --features r8-repeat-accepted
build dg-r5-fixture r5
build dg-r6-fixture-v1 r6-v1
build dg-r6-fixture-v2 r6-v2
mkdir -p target/idl
"${ANCHOR_BIN:-anchor}" idl build -p dg_vault_v1 -o target/idl/v1.json -- --offline
"${ANCHOR_BIN:-anchor}" idl build -p dg_vault_v2 -o target/idl/v2.json -- --offline
"${ANCHOR_BIN:-anchor}" idl build -p dg_r5_fixture -o target/idl/r5.json -- --offline
bash scripts/build-escrow.sh
