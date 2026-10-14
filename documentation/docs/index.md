# Introduction

Doppelganger runs a configured V1 → V2 Solana program migration in LiteSVM and checks the resulting accounts against declared preservation rules, relationships and lifecycle scenarios. The policy-controlled `test` command writes the evidence and returns an exit code suitable for CI.

## Start with a local test

Follow the [quickstart](/docs/quickstart) to install the CLI and run a prepared Vault. Then use [your first migration test](/docs/first-test) to inspect a known defect, capture it and replay it offline. No local validator, API server, access key, wallet funding, Anchor CLI or Solana CLI is needed for that tutorial.

::: warning Distribution status
The configured GitHub remote, `https://github.com/kuroryujinn/doppleganger.git`, returned an empty repository during verification on 12 October 2026. The local checkout contains the implementation, but a public clone cannot yet follow this tutorial. See [installation](/docs/installation) for the exact clone command, the local-checkout path and the publication prerequisite.
:::

## What a result means

A PASS means the supplied inputs met the supplied policy. It does not establish universal program safety. A successful migration transaction can still produce a FAIL if it changes a protected balance, redirects a declared account reference or breaks a lifecycle expectation.

Doppelganger does not deploy or upgrade live programs. Devnet capture is a separate bounded, read-only operation; migration execution and replay remain local.

## Choose a reference

- [Prepared projects](/docs/prepared-projects): files the engine consumes.
- [Complete test workflow](/docs/cli-test): flags, output directories and process exits.
- [Report schema](/docs/report-schema): inspect raw and decoded account evidence.
- [Supported scope](/docs/supported-scope): Anchor/Borsh coverage and resource limits.
- [Prepared-project API](/docs/backend): separate hosted job boundary.

These pages describe CLI 0.1.0. The CLI and prepared-project API are implemented; uploaded-source compilation and the migration-job browser application remain unfinished. This documentation website is a separate static interface.
