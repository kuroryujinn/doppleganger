#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p dg-cli --locked --offline
python3 scripts/package-release.py
