# Prepared project structure

A prepared project supplies compiled SBF programs, matching Anchor IDLs and explicit migration/test configuration. Doppelganger does not infer a migration from Rust source.

## Required inputs

```text
project/
├── doppelganger.toml          # input paths, addresses, fixture, migration, rules
├── doppelganger-policy.toml   # relationships, static approvals, coverage policy
├── v1.bin                    # original program
├── v2.bin                    # candidate program
├── v1.json                   # V1 Anchor IDL
└── v2.json                   # V2 Anchor IDL
```

Those filenames for binaries and IDLs are conventions used by the [quickstart](/docs/quickstart), not reserved engine names. Manifest paths can point elsewhere. The default policy filename is reserved by the production workflow unless `--policy FILE` is supplied.

| Input | Required? | Why |
| --- | --- | --- |
| Manifest | Yes | Select identities, instructions, arguments and observations |
| Two SBF binaries | Yes | Execute both versions locally |
| Two IDLs | Yes | Static comparison and typed runtime bindings |
| Policy | Yes for `test` | Final decision and relationship declarations |
| Fixture definitions | Default state path | Construct V1 through instructions or synthetic fields |
| Frozen snapshot | Optional alternative | Restore captured V1 state with `--snapshot` |
| Alternate fixtures | Optional | Boundary/zero-value lifecycle scenarios |
| Existing output directory | No | `test` requires a new directory and creates it |

## Repository examples

`examples/vault-runtime` references built artifacts under `target/`, which are absent in a fresh checkout. `examples/vault-runtime-devnet` references checked-in artifacts under `fixtures/devnet-snapshots/vault-devnet/artifacts`. The documentation helper copies the latter into a standalone Vault with the former's reviewed policy.

Vault tests account growth and a new withdrawal limit. Escrow exercises multiple parties and an enum state machine. Record is a minimal configuration-only onboarding example using its prepared fixture binaries.

## Relative paths and registration

Paths resolve relative to the manifest, not to the shell's current directory. Local CLI projects may reference files outside the prepared directory. Uploaded prepared ZIPs have a stricter sandbox boundary; do not assume the HTTP API accepts the same external paths.

Registration is optional. `test DIRECTORY` and `test MANIFEST.toml` work directly. The production `test` command does not resolve a registered name; registry-oriented commands are documented in [project commands](/docs/cli-project).

## Copy for your own program

Start from a complete prepared example. Replace artifacts and IDs, review IDL instruction/account bindings, configure the V1 world, then declare target observations and expectations. Run `doctor` followed by `test --json`. A failure is evidence to inspect, not a suggestion to weaken policy.
