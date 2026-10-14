# Frequently asked questions

Answers for local CLI 0.1.0 and its prepared-project workflow.

## Do I need an access key?

No. Local fixture tests and frozen snapshot replay require no API token, paid service or wallet funding. A private RPC provider may require its own credential for live capture. The HTTP API uses operator-provisioned bearer tokens and is a separate surface.

## Do I need Anchor or Solana CLI?

Not to execute prepared binaries and IDLs. The [quickstart](/docs/quickstart) uses checked-in artifacts. Install the pinned build tools only to compile your own programs or rebuild acceptance fixtures; [prerequisites](/docs/prerequisites) links to their official installation guides.

## Do I need a validator or Devnet?

No validator or cluster is needed for local migration execution. LiteSVM runs in the CLI process. Only explicit `snapshot capture` contacts Devnet; frozen replay remains offline.

## Why does my folder contain no executable command?

Source files are not an installed CLI. Use `cargo install --path crates/dg-cli --locked`, or build and invoke `./target/release/doppelganger`. Set `$HOME/.cargo/bin` on PATH. See [installation](/docs/installation).

## Why did the clone return no source?

The configured GitHub remote was empty during verification. The maintainer needs to publish the source before a new developer can clone it. The local checkout and historical release packaging are not proof of public distribution.

## Why use both a manifest and a policy?

The manifest configures state, transaction bindings, observations, preservation and lifecycle scenarios. The policy declares relationships, structural approvals and required coverage, then decides whether evidence is acceptable.

## Does a successful transaction mean PASS?

No. The broken Vault transaction succeeds but loses 50,000 from its protected balance, so policy returns FAIL. See [your first migration test](/docs/first-test).

## Does PASS certify program security?

No. It covers configured states, observations, rules and scenarios. It does not certify undeclared invariants or every possible transaction path.

## Can I provide just V2 Rust source?

No. The current CLI needs prepared V1/V2 binaries and IDLs plus configuration. Hosted source-folder compilation is unfinished. An explicit V1 baseline cannot be reconstructed automatically.

## Why does regression replay pass a failed migration?

Its question is whether the recorded failure reproduces. Portable replay exit 0 means matched evidence. Read the separate migration exit/verdict. Run a normal production test to qualify a corrected candidate.

## Can I host just this documentation for free?

Yes, it builds to static HTML/CSS/JS and requires no Rust backend, Docker, Supabase or access key. The separate `documentation/README.md` contains Vercel/Render static-host settings. Migration-job execution has different hosting requirements.
