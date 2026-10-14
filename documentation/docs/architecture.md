# How the engine works

The production workflow joins static analysis and local execution, then applies policy to the observed evidence. No remote blockchain service executes the migration.

## Execution sequence

```text
manifest + policy + SBF artifacts + Anchor IDLs
                    │
             validate and hash
                    │
               IDL analysis
                    │
         construct or restore V1 state
                    │
          load V2 and run migration
                    │
   preservation + relationships + lifecycle
                    │
            evaluate declared policy
                    │
       JSON / HTML / Markdown + exit code
```

Lifecycle cases construct fresh worlds rather than mutating the main migration's AFTER state. Frozen snapshot runs restore the sealed state instead of silently replacing it with fixture initialization.

## Crate responsibilities

| Crate | Responsibility |
| --- | --- |
| `dg-config` | Parse manifests, normalize instruction bindings, resolve named addresses and encode arguments |
| `dg-analysis` | Normalize Anchor IDLs and compare account/interface structure without a VM |
| `dg-state` | Raw snapshots, generic fixture materialization and typed Borsh decoding |
| `dg-runtime` | LiteSVM V1 worlds, local V2 swap, transactions, preservation and lifecycle execution |
| `dg-policy` | Pure declared-relationship evaluation and final upgrade decisions |
| `dg-snapshot` | Bounded finalized Devnet reads, snapshot integrity and local restoration |
| `dg-regression` | Native finding/scenario cases and candidate reproduction |
| `dg-cli` | Commands, production orchestration, report output and portable whole-workflow cases |

This table describes responsibilities, not a dependency chain. In particular, portable capture/replay is implemented by `dg-cli` while native cases use `dg-regression`.

## Evidence and policy

A transaction outcome is one input to policy. Static findings, preservation evaluations, graph edges, lifecycle outcomes and declared coverage also contribute. Unexpected structural changes block unless exactly approved; runtime failures cannot be waived by static policy. [Migration evidence](/docs/evidence) shows how to inspect each layer.

## Devnet is a separate operation

`snapshot capture` validates Devnet genesis identity and uses finalized read-only RPC calls for explicitly configured accounts. `test --snapshot` and snapshot replay make no RPC calls. The local runner does not deploy programs, discover all related accounts or scan mainnet.
