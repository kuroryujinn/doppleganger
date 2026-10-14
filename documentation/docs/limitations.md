# Known limitations

Use the tool within the implemented and qualified boundaries. A PASS result applies to the observed world and declared policy.

## Implemented surfaces

- Local CLI static analysis, generic prepared migration execution, preservation, declared relationships, lifecycle, policy and reports.
- Bounded read-only Devnet acquisition, integrity-checked frozen restoration and offline replay.
- Native finding/scenario regressions and portable complete-workflow reproduction.
- Prepared-project API and isolated durable worker, locally exercised against rootless Docker.

This documentation site is implemented separately. It does not provide a browser application for uploading and executing migration jobs.

## Unfinished product capabilities

Uploaded-source compilation, drag-and-drop migration frontend, public signup/billing, automatic account discovery, mainnet scanning, custom invariant expressions, token/economic conservation packs, automatic upgrades and deployment are not implemented. Do not treat roadmap documentation as a released capability.

## Qualification boundaries

Linux x86_64/WSL2 is qualified. Other hosts need separate validation. Anchor/Borsh support is the documented subset, not universal compiler compatibility. Declared graph edges cover only observed accounts and configured targets. Lifecycle checks cover explicit scenarios, not exhaustive paths.

Frozen captures are historical state, not proof that live Devnet accounts still match. Public deterministic test identities must never hold real-value assets.

## Distribution and hosting

The configured public GitHub remote currently advertises no refs and clones empty; no public release archive is verified. This blocks the fresh public checkout path. The source installation and tutorial were exercised locally, and the docs describe this distribution gap explicitly.

The local CLI accepts developer-controlled artifact paths. Uploaded projects instead require the API worker's container isolation. A normal web service hosting the API is not equivalent to provisioning the required rootless Docker execution host.

## Command compatibility

`project analyze/check` retain the legacy project schema, even though `project add` accepts current manifests. `migration check` remains Vault-demo-specific. Runtime and snapshot replay commands are evidence-only. Use production `test` for final policy gating.
