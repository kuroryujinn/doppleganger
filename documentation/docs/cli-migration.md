# Migration commands

Inspect transaction bindings without execution, or invoke the complete production workflow through the `migration test` spelling. The older `check` command remains demo-specific.

## Inspect

```text
doppelganger migration inspect [MANIFEST_OR_DIRECTORY_OR_REGISTERED_NAME]
```

Default input is `doppelganger.toml` in the working directory. No flags are accepted. It validates the prepared project, resolves addresses and fixture instruction plans, and prints the migration instruction, discriminator, IDL account order, signer/writable flags and encoded argument sizes. It writes no report and submits no transaction.

```bash
doppelganger migration inspect runs/vault-project
```

Expected fixture line: `Fixture plan: 1 initialization, 1 setup instructions (not executed)`. Success exits 0; invalid bindings/configuration fail through the retained command error path.

## Complete test alias

```text
doppelganger migration test PROJECT [TEST_OPTIONS]
```

```bash
doppelganger migration test runs/vault-project --ci --output runs/vault-alias
```

This is exactly the same production pipeline as `test`, including defaults, reports and exits. All flags are listed in [complete test workflow](/docs/cli-test).

## Legacy check

```text
doppelganger migration check [MANIFEST_OR_DIRECTORY_OR_REGISTERED_NAME]
```

This accepts only the retained empty `vault` demo fixture and bundled program/type constraints. Generic production fixtures intentionally receive an unsupported-fixture error. After building the original demo artifacts:

```bash
doppelganger migration check examples/vault-migration
```

It writes `report/<project>-migration.json` and HTML and exits 1 on a failed demo check. Do not substitute this command for `test` on a new project. The generic quickstart was verified using `test`; legacy demo execution was not rerun for this documentation.
