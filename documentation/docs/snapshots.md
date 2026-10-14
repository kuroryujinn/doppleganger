# Devnet capture and offline replay

Acquire a bounded set of explicitly configured Devnet accounts once, then replay that V1 state locally. Capture and migration execution are separate operations.

## Configure acquisition

The tracked Vault manifest contains:

```toml
[snapshot]
source = "devnet"
rpc_url_env = "DG_DEVNET_RPC_URL"

[[snapshot.accounts]]
name = "vault"
address = "$vault"

[[snapshot.accounts]]
name = "owner"
address = "$owner"
owner = "$system_program"
```

Choose `rpc_url` or `rpc_url_env`, not both. With neither, the adapter uses the public Devnet endpoint. This example explicitly names an environment variable, so it must be set for **capture**. Connection settings are not copied into evidence.

Optional `min_context_slot`, `max_account_bytes`, per-account `account_type`/`optional`, and configured local program entries are described in [configuration reference](/docs/configuration).

## Capture live accounts

```bash
export DG_DEVNET_RPC_URL=https://api.devnet.solana.com
doppelganger snapshot capture examples/vault-runtime-devnet --output runs/live-capture
```

This contacts Devnet. It requires those accounts to exist with the expected V1 identity/ownership; it does not create them. The importer verifies genesis identity, finalized commitment and configured program/account identities. It uses read-only genesis/account calls and sends zero remote transactions.

Capture is bounded to one finalized batch of at most 100 configured accounts, 16 MiB aggregate raw bytes and configured per-account limits. No discovery, mainnet scanning or wallet funding is performed. A private RPC provider may require its own credential in the endpoint; the CLI has no general access-key requirement.

Live recapture was not executed for this documentation. Historical live acceptance is retained in the repository; do not mistake it for present account availability.

## Offline replay

The repository contains a genuine frozen Vault capture at slot 507756125, with its matched V1 program and good/broken V2 candidates.

```bash
export DG_DEVNET_RPC_URL=http://127.0.0.1:1
doppelganger snapshot inspect fixtures/devnet-snapshots/vault-devnet

doppelganger test examples/vault-runtime-devnet \
  --policy examples/vault-runtime/doppelganger-policy.toml \
  --snapshot fixtures/devnet-snapshots/vault-devnet \
  --ci --output runs/offline-vault
```

The unusable endpoint demonstrates that this path does not require RPC. It was executed successfully. Imported state is restored; fixture initialization/setup cannot silently replace it. Hashes, schema, program/IDL identities and controlled local signer consistency must match.

## Compare a candidate on the same state

```bash
doppelganger snapshot replay fixtures/devnet-snapshots/vault-devnet \
  --project examples/vault-runtime-devnet --lifecycle \
  --candidate fixtures/devnet-snapshots/vault-devnet/artifacts/v2-broken.bin \
  --output runs/frozen-broken.json
```

Inspect preservation output rather than the evidence command's process status. Both candidates use the same immutable V1 snapshot. For a final policy verdict, prepare a manifest selecting your V2 candidate and run `test --snapshot` with its reviewed policy.

Never edit sealed snapshots or their hashes to make a run pass. Native regression cases retain source path references; the old retained acceptance-machine case may need recapturing on another checkout.
