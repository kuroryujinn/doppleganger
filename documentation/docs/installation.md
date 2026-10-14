# Installation

Install the executable named `doppelganger` from the Cargo package named `dg-cli`. Run the following inside Linux/WSL.

## Obtain the source

This is the configured repository URL:

```bash
git clone https://github.com/kuroryujinn/doppleganger.git
cd doppleganger
```

::: warning Public checkout currently blocked
The clone command completed but returned an empty repository on 12 October 2026; `git ls-remote` advertised no refs. The maintainer must publish the source, tracked fixture binaries and this documentation before the public workflow is usable. No downloadable GitHub release is verified.
:::

If you already have the supplied local project, open its directory instead. On the verified development host:

```bash
cd /home/kuro/doppleganger
```

On another machine use your actual checkout location. Confirm the files exist before continuing:

```bash
ls Cargo.toml crates/dg-cli/Cargo.toml
ls fixtures/devnet-snapshots/vault-devnet/artifacts
```

## Install the command

After completing [prerequisites](/docs/prerequisites), from the repository root:

```bash
cargo +1.98.0 install --path crates/dg-cli --locked
export PATH="$HOME/.cargo/bin:$PATH"
doppelganger --version
doppelganger --help
```

Expected version output:

```text
doppelganger 0.1.0
```

`--locked` preserves the checked-in dependency resolution. The first source build downloads Cargo dependencies and can take several minutes. Do not add `--offline` on a fresh dependency cache.

## Build without installing

```bash
cargo +1.98.0 build --release -p dg-cli --locked
./target/release/doppelganger --version
```

Use `./target/release/doppelganger` in place of `doppelganger` in later commands. Merely being in the repository directory does not put this executable on PATH.

## Check a project

`doctor` reports platform and tool availability and validates a prepared manifest, its artifacts, IDLs, policy and deterministic signer configuration. It performs no migration and no RPC request.

```bash
doppelganger doctor runs/vault-project --json
```

Create that directory in the [quickstart](/docs/quickstart) first. Missing Anchor/Solana tools are warnings; existing prepared artifacts can still run. `doctor` is a preflight check, not evidence that the V2 migration succeeds.

## Local release archives

The repository can package a Linux archive using `scripts/package-release.sh` after qualification artifacts and example builds exist. This is a local packaging script, not a published distribution. An archive carries `bin/doppelganger`, prepared examples and `SHA256SUMS`; verify its adjacent checksum and run `sha256sum -c SHA256SUMS` inside the extracted directory. Do not expect `report/releases/` to exist in a fresh clone.

For PATH, toolchain and missing-artifact errors, see [troubleshooting](/docs/troubleshooting).
