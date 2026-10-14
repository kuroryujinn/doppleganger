# Getting started — 0.1.0

Doppelganger qualifies a prepared Anchor V1 → V2 migration locally. The MVP ends at reproducible policy-controlled reports; APIs, source-folder upload and frontend are the next phase.

## Install

The qualified target is Linux x86_64, including WSL2. The versioned local archive is produced by `bash scripts/package-release.sh` in `report/releases/`. Extract it in a new directory, check its adjacent `.sha256`, and run `sha256sum -c SHA256SUMS` inside the extracted directory. Add its `bin` directory to PATH. Prepared example execution needs no Anchor CLI, Solana CLI, Rust compiler or cluster.

From a source checkout with Rust and the locked dependencies:

```bash
cargo install --path crates/dg-cli --locked
export PATH="$HOME/.cargo/bin:$PATH"
doppelganger --version
```

To build the bundled Solana artifacts, provision platform-tools v1.57, Solana CLI 3.1.10 and Anchor CLI 1.2.0, then run:

```bash
bash scripts/build-production-fixtures.sh
```

Set `ANCHOR_BIN` to the Anchor 1.2.0 executable if your version-manager wrapper selects another tool. Build scripts use cached tools/dependencies in offline mode; provision the cache before the run.

## Run the standalone release examples

Inside an extracted release:

```bash
bin/doppelganger doctor examples/vault --json
bin/doppelganger test examples/vault --ci --output runs/vault
bin/doppelganger test examples/escrow --ci --output runs/escrow
bin/doppelganger test examples/record --ci --output runs/record
```

In a source checkout the corresponding directories are `examples/vault-runtime`, `examples/escrow-runtime` and `examples/record-runtime`.

Read `report.json` for evidence, `report.html` for a browser view, and `summary.md` for the verdict. The exit code is authoritative: 0 pass, 1 policy violation, 2 configuration, 3 runtime/evaluation, 4 internal/output failure. Choose a fresh output directory for every run.

## Onboard your own project

1. Build both SBF artifacts and IDLs. V1 and V2 must have the same declared program ID.
2. Copy a prepared example directory into your project. Change only its manifest and policy plus its artifacts; no engine code or registration branch is needed.
3. Configure addresses, fixture initialization/setup or explicit synthetic state, instruction arguments and account order bindings.
4. Map V1/V2 account types in `migration_targets`; declare preservation, relationships and lifecycle expectations.
5. Run `doctor` to validate the files and profile, then `test --json` to inspect the static differences. Review differences before adding exact expected-change approvals. A failed first run is useful evidence, not an instruction to approve every difference.
6. Repeat with a fresh output directory, capture failures, and add the command to CI.

The [manifest guide](project-manifest.md), [engine reference](engine-reference.md), [policy/workflow guide](production-workflow.md) and [supported formats](supported-anchor.md) describe every supported configuration surface. The Record example is a minimal configuration-only onboarding case; Vault and Escrow demonstrate materially different executed upgrade layouts.

## Reproduce any complete production failure

```bash
doppelganger regression capture --from-report runs/broken/report.json --store cases/broken
doppelganger regression replay cases/broken --output runs/replay
```

The case copies all required inputs, checks their hashes, verifies the original normalized result at capture time, and verifies it again on replay. Move the entire case directory to another local checkout or machine with the same CLI version; source-project paths are unnecessary. Replay exits 0 when the recorded failure is reproduced and 1 on a mismatch. Invalid/tampered cases exit 2. Full workflow cases include static and relationship findings. Existing native `regression create/reproduce` remains available for individual preservation/lifecycle findings and candidate-fix overrides; see the [regression reference](engine-reference.md#regression-cases).
