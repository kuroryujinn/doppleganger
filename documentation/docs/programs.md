# V1 and V2 programs

The CLI consumes compiled Solana SBF ELF programs and Anchor IDLs. It loads V1 into a local LiteSVM world, then replaces the program at the same ID with V2 before executing the configured migration.

## Prepared inputs need no build tools

The [quickstart](/docs/quickstart) copies the repository's tracked acceptance binaries. Their `.bin` extension is a packaging choice; the engine reads ELF bytes. Anchor and Solana CLI are not needed to execute those artifacts.

For your own project, provide compiled V1 and V2 plus matching IDLs. The tool does not reconstruct V1 from only a V2 source folder.

## Rebuild the source acceptance examples

Install the required tools using the upstream guides:

- [Anchor installation](https://www.anchor-lang.com/docs/installation), selecting CLI 1.2.0.
- [Solana installation](https://solana.com/docs/intro/installation), selecting CLI 3.1.10 and platform-tools v1.57.

Verify `anchor --version`, `solana --version` and `cargo build-sbf --version`. Once the tools and dependency caches are provisioned, run from the repository root:

```bash
bash scripts/build-production-fixtures.sh
```

The script builds good/broken Vault and Escrow artifacts and the Record fixtures, and generates IDLs. It uses `--offline --skip-tools-install`; it is **not** a bootstrap script for a blank machine. Cache provisioning was not verified on a fresh machine. The prepared-artifact tutorial avoids this dependency entirely.

If an AVM wrapper selects the wrong tool, set `ANCHOR_BIN` to the absolute installed Anchor 1.2.0 executable before running the script. Do not copy the maintainer's `/home/kuro/...` path onto another machine.

## Live deployment is separate

The CLI does not submit upgrade transactions, create a mainnet deployment or sign remotely. A configured migration is executed only in the local world. Frozen Devnet fixtures retain historical provenance but replay is offline.
