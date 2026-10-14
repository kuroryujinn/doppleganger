# Your first migration test

Use the prepared Vault from the [quickstart](/docs/quickstart). This walkthrough connects its configuration to real BEFORE/AFTER evidence, a broken V2 candidate and a replayable regression case.

## 1. Read the inputs

`runs/vault-project/doppelganger.toml` contains these real bindings:

```toml
schema_version = 1

[versions.v1]
program_path = "v1.bin"
idl = "v1.json"
program_id = "DTbPK89xmJH65ay3YyJTZnPnDWsyXU3KNLiEvXQ7bjoT"

[versions.v2]
program_path = "v2.bin"
idl = "v2.json"
program_id = "DTbPK89xmJH65ay3YyJTZnPnDWsyXU3KNLiEvXQ7bjoT"
```

These are excerpts, not a complete manifest. Both versions use the same declared ID. Paths resolve relative to the manifest directory. The helper copies the checked-in Devnet acceptance binaries, but running them here creates a **local fixture world**, not a Devnet transaction.

```toml
[[fixture.initialize]]
instruction = "initialize"
account_mapping = { vault = "vault", owner = "owner", system_program = "system_program" }

[[fixture.setup]]
instruction = "deposit"
account_mapping = { vault = "vault", owner = "owner", system_program = "system_program" }
args = { amount = "u64:1000000" }

[migration]
instruction = "migrate"
account_mapping = { vault = "vault", owner = "owner", system_program = "system_program" }
```

Initialization creates the Vault. Setup deposits 1,000,000 lamports into its recorded balance. The IDL supplies account order, signer/writable flags, discriminators and argument encoding. Local seeded test identities sign the transactions.

```toml
[[preservation]]
id = "balance-field"
account = "vault"
field = "balance"
rule = "equal"
```

This rule requires equal decoded balance values before and after migration. The separate policy also declares authority, program ownership and PDA checks:

```toml
schema_version = 1

[[relationship]]
id = "vault-authority"
kind = "authority"
account = "vault"
target = "$owner"
field = "owner"
```

The full profile includes exact reviewed static-change approvals. It is specific to this fixture upgrade; do not reuse it as blanket approval for another program.

## 2. Observe the V1 world

```bash
doppelganger runtime build runs/vault-project --output runs/v1-world.json
```

The output confirms two funded signers, one initialization, one setup action and one observed Vault. The account has 48 bytes of data and decodes as `VaultV1`. This command does not migrate or evaluate policy.

The production `test` command builds the same default world internally. For frozen restoration instead, supply `--snapshot`; see [offline replay](/docs/snapshots#offline-replay).

## 3. Establish the known-good result

```bash
doppelganger test runs/vault-project --ci --output runs/vault-good-check
```

Expect PASS and exit 0. BEFORE and AFTER are recorded in `execution.before_accounts` and `execution.after_accounts`. `execution.transaction.success` records the migration transaction outcome; `policy.passed` records the wider decision.

## 4. Select the existing broken candidate

The checked-in `v2-broken.bin` was built with the repository's `r7-broken-balance` feature. It reduces the balance during migration. Prepare a separate directory using the same manifest, V1, IDLs and policy:

```bash
python3 documentation/scripts/prepare-example.py \
  --output runs/vault-broken-project --candidate broken

doppelganger test runs/vault-broken-project --ci --output runs/vault-fail
printf 'Exit: %s\n' "$?"
```

Verified output:

```text
DOPPELGANGER MIGRATION TEST
Result: FAIL
Exit code: 1
Report: runs/vault-fail/report.json
Exit: 1
```

This is an expected policy failure. The CLI completed the evaluation and wrote reports; its nonzero exit correctly blocks CI.

## 5. Read the failed check

```bash
python3 - <<'PYCODE'
import json
from pathlib import Path
report = json.loads(Path('runs/vault-fail/report.json').read_text())
print('Transaction succeeded:', report['execution']['transaction']['success'])
for finding in report['preservation']['findings']:
    if finding['id'] == 'PRESERVE-balance-field':
        print(json.dumps(finding, indent=2))
PYCODE
```

The real finding is `PRESERVE-balance-field`, category `FIELD_NOT_PRESERVED`, status FAIL:

```json
{
  "before": {"state": "found", "value": {"kind": "integer", "value": "1000000"}},
  "after": {"state": "found", "value": {"kind": "integer", "value": "950000"}},
  "rule": "equal",
  "status": "FAIL"
}
```

This is a selected excerpt. The transaction succeeded, but it lost 50,000 from the protected balance. The size/owner/address rules cannot substitute for that balance check. Do not approve this runtime defect as an expected static change.

## 6. Capture the complete failure

```bash
doppelganger regression capture --from-report runs/vault-fail/report.json \
  --store runs/vault-case

doppelganger regression replay runs/vault-case --output runs/vault-replay
```

Capture copies the evaluated inputs, checks their hashes and verifies the normalized failure again. Replay returns `REPRODUCED`, `matched: true`, `migration_exit_code: 1`, and process exit 0. Exit 0 means **the recorded failure reproduced**. It does not mean the migration passed.

Move the entire case directory to another machine with the same CLI. The portable case is independent of the original source artifact paths. Leave its hashed files unchanged.

## 7. Check a corrected candidate

Portable replay has no candidate override. Use a native individual finding case for that comparison:

```bash
doppelganger regression create --from-report runs/vault-fail/report.json \
  --manifest runs/vault-broken-project --finding PRESERVE-balance-field \
  --store runs/native-cases
```

The JSON response reports the new case directory. Substitute that exact value for `CASE_DIRECTORY`:

```bash
doppelganger regression reproduce CASE_DIRECTORY

doppelganger regression reproduce CASE_DIRECTORY \
  --candidate runs/vault-project/v2.bin --candidate-idl runs/vault-project/v2.json
```

The broken candidate returns `REPRODUCED`; the good candidate returns `NOT_REPRODUCED`. Both native commands can exit 0, so inspect their JSON result. Native cases retain source artifact references: keep the project paths available.

Finally run the complete `test` workflow against the corrected project and its reviewed policy. Removing one regression does not establish that every other required check passes.
