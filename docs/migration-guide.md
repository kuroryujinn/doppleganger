# Migrating to 0.1.0 production workflow

Add `schema_version = 1` before any TOML table in each current `doppelganger.toml`. Existing pre-versioned manifests remain the frozen legacy format and are reported explicitly; adding the version makes the current contract unambiguous. Unsupported explicit versions fail. Frozen regression/snapshot manifests should not be edited: their original hashes are part of their identity.

Add a separate `doppelganger-policy.toml` with schema_version 1, exact reviewed expected-change approvals and relationship declarations. Replace deployment gating based on legacy `runtime verify/lifecycle` or `migration check` with `doppelganger test PROJECT --ci`. Those legacy commands retain their original evidence/demo interfaces. Generic production execution works through `test` for all supported projects.

Registry map-only files are recognized as the original legacy registry. The next validated update preserves that file as a backup and writes version 1 under the shared lock; no entries are dropped. Unknown versions/corruption block operations. `project recover` restores the prior validated state and retains damaged bytes.

Native preservation/lifecycle regression cases keep their original schema and APIs. Use `regression capture/replay` for a new portable whole-workflow case or a static/relationship policy failure. Both forms retain exact artifact/state evidence; replay semantics differ from migration policy success, as documented in the getting-started guide.
