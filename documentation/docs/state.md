# Fixtures and snapshots

The runtime needs an explicit V1 account world. Use instruction-based fixtures, explicitly synthetic fields or a frozen snapshot. These sources have different provenance.

## Instruction-created fixtures

The Vault default observes an account that V1 will create:

```toml
[fixture]
name = "vault-runtime"
payer = "owner"

[[fixture.account]]
name = "vault"
type = "VaultV1"
address = "$vault"
owner = "$program"
lamports = 0
```

No `fields` table is supplied. The following `fixture.initialize` and `fixture.setup` instructions create and fund it locally. `runtime build` records actions, raw snapshots and decoded observations without migration.

## Synthetic fields

Supplying `[fixture.account.fields]` materializes explicit typed test state through the IDL/Borsh encoder. It must supply supported fields matching the declared account type. Synthetic state is labelled as test construction and need not have been created by an on-chain constructor.

Named alternate fixtures under `fixtures` can feed zero-value/boundary lifecycle cases. Each scenario gets a fresh world. See [lifecycle testing](/docs/lifecycle).

## Frozen restoration

```bash
doppelganger test examples/vault-runtime-devnet \
  --policy examples/vault-runtime/doppelganger-policy.toml \
  --snapshot fixtures/devnet-snapshots/vault-devnet \
  --ci --output runs/frozen-vault
```

From the repository root, this restores the retained genuine Devnet V1 state and runs the complete local workflow. No RPC is required. Snapshot content hash, schema, program/IDL identity, configured accounts and matching local signers are checked. Fixture initialization cannot silently overwrite imported state.

The main [quickstart](/docs/quickstart) intentionally uses fixture construction. The same checked-in binary can run against either that local fixture or sealed historical state, but the resulting provenance differs.

## Capture is read-only

Live Devnet capture performs bounded finalized RPC reads for configured accounts; it does not discover accounts automatically. Keep immutable snapshots unchanged. See [capture and replay](/docs/snapshots) for connection settings and limits.
