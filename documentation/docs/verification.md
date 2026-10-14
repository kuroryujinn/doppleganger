# Verification and release status

This documentation describes the implementation inspected and exercised on 12 October 2026, on Linux x86_64 under WSL2. Qualification claims are bounded to the checks below.

## Verified local walkthrough

The source installation used Rust/Cargo 1.98.0 and `cargo install --path crates/dg-cli --locked` with a temporary installation root. The installed binary was then used to execute the tutorial helper, doctor, static analysis, all runtime actions, migration inspect, production test and its alias.

The good fixture passes seven preservation rules, three relationships and seven lifecycle scenarios, with nine exact expected static findings. The tracked broken candidate succeeds as a transaction but fails `PRESERVE-balance-field`: balance 1,000,000 → 950,000, exit 1. A nonexistent input produces ERROR/exit 2.

Portable failure capture/replay, native capture/inspect/list/reproduction, corrected-candidate comparison, frozen snapshot inspection/replay and production snapshot testing were executed. Offline paths were run with an unusable RPC endpoint. Registry mutation tests used a temporary working directory; current-versus-legacy schema behavior was checked explicitly.

Machine-readable command/output evidence is retained in `documentation/verification/cli-results.json`. A rerunnable verification script is included:

```bash
python3 documentation/scripts/verify-cli.py --binary doppelganger
```

It uses temporary projects and leaves the user registry unchanged. It writes static analysis under the selected working directory's `.doppelganger` output area. A full command matrix and website checks are recorded in `documentation/VERIFICATION.md`.

## Website verification

The independent Markdown/VitePress site has static routes under `/docs`, collapsible navigation, breadcrumbs, generated TOC, previous/next links, local full-text search, copyable highlighted code and light/dark themes. Build and link checks plus browser interaction/layout checks are recorded in the verification report. No migration engine/protocol changes were made for this frontend.

## Blocked checks

The real configured GitHub clone returned an empty repository; `git ls-remote` advertised no branches or tags. A fresh public clone cannot install the project until the maintainer publishes source and fixtures. No public release archive, cloud deployment or hosted GitHub Actions run was verified.

System package/Rust bootstrap was not rerun on a blank machine. Source installation used an already provisioned compiler/linker and cached dependencies. Live Devnet recapture and fresh Anchor/Solana tool provisioning were not run; commands/requirements were checked against source and the existing accepted fixtures.

## Historical evidence

Root docs include the local 0.1.0 MVP acceptance, R11 genuine Devnet capture and later prepared-project backend acceptance. Some older status pages predate the implemented API. These pages prefer current source and executed workflows over old roadmap claims. Public distribution remains separate from local implementation success.
