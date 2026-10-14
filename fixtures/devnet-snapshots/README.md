# Snapshot Fixtures

`escrow-mock/snapshot.json` is a deterministic mock-RPC capture of local R5
state. It is not evidence of a Devnet deployment. The public test identities,
mock origin, slot 42, zero capture time, and `acquisition=mock_rpc` make this
distinction explicit. No credentials or private wallet keys are present.

It tests deserialization, integrity, exact VM restoration, candidate comparison,
R6/R7/R8 reuse, and offline R10 reproduction. Do not overwrite it with remote
state or relabel its acquisition. Real Devnet captures belong in a separate
immutable directory and must retain their actual provenance.

`vault-devnet/` is the separate **real Devnet** R11 acceptance capture. It
retains public setup evidence, exact local artifacts, same-snapshot candidate
reports, and a snapshot-pinned R10 regression. See its README for offline
reproduction; never replace or relabel the Escrow mock with live state.
