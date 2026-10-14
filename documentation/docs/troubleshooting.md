# Troubleshooting

Run `doctor` on the actual prepared project first. It does not execute migration or require a remote endpoint.

```bash
doppelganger doctor runs/vault-project --json
```

## Installation and shell problems

| Symptom | What to do |
| --- | --- |
| `doppelganger: command not found` | Run `cargo install --path crates/dg-cli --locked` from the source root and export `$HOME/.cargo/bin` on PATH, or use `./target/release/doppelganger` |
| `cargo: command not found` | Install Rust with rustup and source `$HOME/.cargo/env` in the Linux shell |
| Clone returns empty repository | Source has not been published at the configured remote; use the supplied local checkout until publication |
| Missing `target/deploy` artifacts | These are generated, not cloned; use the checked-in artifact quickstart or provision/build the fixtures |
| Windows path/tool appears in WSL | Check `command -v cargo` and `command -v doppelganger`; use Linux executables from the WSL filesystem |
| Linker `cc` is missing | Install Ubuntu `build-essential` |
| Offline build cache miss | Fetch/provision tools and dependencies before using offline build scripts |

Being in the project folder does not automatically expose a binary as a command. `/path/to/project` and `CASE_DIRECTORY` are explanatory placeholders; use the real tutorial directory or case path returned by the CLI.

## Production results

| Symptom | Diagnosis/correction |
| --- | --- |
| Exit 1, unexpected static finding | Review exact structural changes; approve only intended selectors/values |
| Exit 1, preservation/relationship/lifecycle failure | Read BEFORE/AFTER and transaction logs; correct migration or the declared expectation |
| Exit 2, missing policy/artifact/IDL | Fix manifest-relative files; supply `--policy` when no sibling profile exists |
| Exit 2, unknown field/version | Use current schema and supported fields; do not edit sealed case hashes |
| Exit 3, setup/evaluation error | Check V1 initialization, funding, account observations and typed decoding |
| Exit 4, output already exists | Choose a fresh `--output` directory |
| Exit 4, input mutation | Freeze inputs during the run and repeat with new output |

## State and regression problems

| Symptom | What to do |
| --- | --- |
| Snapshot signer/identity mismatch | Use the matching controlled fixture signer and original program/IDL identities |
| Capture connection environment variable unset | Set the variable named by `rpc_url_env`; offline replay does not need it |
| Portable replay integrity error | Restore the entire original case directory with its exact inventory/hashes |
| Native reproduction can't find source artifacts | Keep original paths valid or capture a portable case for transfer |
| Replay exits 0 but migration failed | Expected: replay matched the stored defect; inspect `migration_exit_code` |
| Current registered manifest fails `project analyze` | That command still reads legacy schema; use direct `analyze` |
| Corrupt registry | Preserve diagnostics, use `project recover` if a validated backup exists, re-register a lost latest entry |

Missing Anchor/Solana availability in `doctor` is a warning for prepared execution. Their official installation links are on [prerequisites](/docs/prerequisites), with the qualified versions. No general access key is required for local testing.
