# Prerequisites

The qualified host is Linux x86_64, including Ubuntu under WSL2. Run commands inside the Linux terminal and keep the checkout on the Linux filesystem. macOS, ARM Linux and native Windows are not separately qualified by this release.

## For the first local test

| Dependency | Purpose | Requirement |
| --- | --- | --- |
| Git | Obtain the source | Available in the Linux shell |
| Rust and Cargo | Compile the CLI | 1.98.0 is the tested toolchain; no lower MSRV is declared |
| C compiler and linker | Link Rust dependencies | Ubuntu `build-essential` |
| Python | Copy the prepared example and inspect JSON | Python 3.8+ for the tutorial helper |
| curl and CA certificates | Install Rust over HTTPS | Needed only for bootstrap |

On Ubuntu/WSL:

```bash
sudo apt update
sudo apt install -y git curl ca-certificates build-essential python3
uname -sm
git --version
python3 --version
```

The host check should report `Linux x86_64`. Follow [Rust's installation instructions](https://doc.rust-lang.org/book/ch01-01-installation.html), then select the qualified version:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
rustup toolchain install 1.98.0 --profile minimal
rustc +1.98.0 --version
cargo +1.98.0 --version
```

The source build below selects this version explicitly. Installing Rust/system packages on a blank machine was not repeated during this verification; the tested WSL host already supplied the compiler and linker.

## When Anchor and Solana are needed

The first tutorial uses checked-in SBF binaries and IDLs. It does **not** build a Solana program. Install additional tools only to compile your own V1/V2 artifacts or rebuild the repository acceptance fixtures:

- [Official Anchor installation guide](https://www.anchor-lang.com/docs/installation): use Anchor CLI **1.2.0** for the qualified fixtures.
- [Official Solana CLI installation guide](https://solana.com/docs/intro/installation): the tested CLI is **3.1.10**, with SBF platform-tools **v1.57**.

Those upstream guides may default to newer versions. Match the versions above when reproducing this repository's build acceptance. Verify after installation:

```bash
anchor --version
solana --version
cargo build-sbf --version
```

The fixture build scripts assume already cached dependencies and platform tools. [V1 and V2 programs](/docs/programs) explains that boundary.

## Website maintenance is separate

Node.js 22+ and npm are needed to build this documentation site, not to use the CLI. Docker is needed by the isolated prepared-project API worker, not by local CLI tests.
