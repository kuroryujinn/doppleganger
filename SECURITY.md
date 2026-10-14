# Security policy

The supported release line is 0.1.x. This tool tests prepared, developer-controlled projects locally. It sends no upgrade, migration or funding transaction to a cluster during qualification. Snapshot import uses read-only RPC calls.

Do not include production private keys in fixtures. `test_seed` creates public, reproducible test identities and is suitable only for local fixtures and explicitly controlled test deployments.

Report a suspected vulnerability privately to the repository maintainers using the repository host's private vulnerability reporting channel when enabled. If that channel is unavailable, contact the maintainer privately before publishing sensitive details. Include the CLI version, relevant diagnostic, minimal reproducer and affected hashes; remove credentials and private source first. No public disclosure response time or external security audit is claimed for this initial release.

Input budgets are documented in `docs/supported-anchor.md`. Recursive schemas, malformed Borsh tags, unknown format versions, snapshot tampering and artifact changes must fail explicitly. Configured artifact paths may point outside the project directory: the local CLI is a developer tool, not a filesystem sandbox. A future upload/API service must enforce an isolated workspace and execution limits before accepting hostile projects.

Run `cargo audit` with the current RustSec database before distributing a release. Qualification on 2026-10-08 found no known vulnerable locked dependencies after updating rustls to 0.23.45. Five upstream maintenance warnings remain: ansi_term, bincode, derivative, libsecp256k1 and paste. They are transitive dependencies of the pinned Solana/Anchor/VM stack; see `docs/security-qualification.md` for the review. This scan is not a proof of absence of vulnerabilities.
