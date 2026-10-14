# Project commands

The registry records canonical manifest locations in `.doppelganger/projects.json` under the current working directory. Registration is optional for the recommended direct-path workflow.

## Add and list

```text
doppelganger project add MANIFEST_OR_DIRECTORY
doppelganger project list
```

```bash
doppelganger project add runs/vault-project
doppelganger project list
```

`add` accepts a current `doppelganger.toml` directory or a legacy project file and uses its validated project name. It prints `Added project: vault-runtime`. `list` prints name/path pairs. Success exits 0; validation, duplicate-name/path, corrupt-registry and filesystem errors fail through the legacy error path.

## Legacy analysis and checks

```text
doppelganger project analyze NAME [--format text|json|html]
doppelganger project check NAME [REPORT_JSON]
```

::: warning Current schema boundary
`project analyze` and `project check` read the legacy schema (`name`, `program_id`, `v1`, `v2`, `accounts`, optional `scenario`). Registering a current production manifest does not make these commands compatible with it. This mismatch was reproduced. Use direct `analyze` and `test DIRECTORY` for current projects.
:::

After rebuilding the original fixtures, a real legacy example is:

```bash
doppelganger project add examples/vault-schema-only.toml
doppelganger project analyze vault-schema-only --format html
doppelganger project check vault-schema-only
```

This example references `target/idl` and cannot run on a fresh checkout before artifact generation. The reference behavior was tested using an equivalent temporary legacy manifest pointing at the tracked IDLs.

`project analyze` has the same report/format behavior as [static analysis](/docs/cli-analysis). `project check` defaults to `report/<name>.json` and companion HTML. Without a scenario it reports schema-only coverage and no runtime execution. With `scenario = "vault"` it uses the retained Vault demo. It does not apply production policy or required coverage; a schema-only completion may exit 0.

## Registry recovery

```text
doppelganger project recover
```

This explicit command restores the previous validated `.bak` registry and retains corrupted bytes as `projects.json.corrupt.<pid>`. Recovery can lose the most recent registration; add that entry again. It does not repair missing artifacts or changed project names.

Registry writes use file locks and atomic replacement. This is a single-host registry, not a distributed queue. Recovery syntax and behavior were inspected in source; no existing user registry was deliberately corrupted for this documentation.
