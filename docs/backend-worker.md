# Durable worker implementation

The B1 worker is implemented for prepared projects. It adds `dg-worker`, the container-only `dg-sandbox`, a fixed bounded output protocol, a local image assembly recipe, and lifecycle tests. The subsequent B2 [prepared-project API](backend-api.md) now provides HTTP reception and owner-scoped lifecycle/download routes. Source-folder compilation belongs to B3.

**Full live container qualification is pending.** This WSL installation now has [native rootless Docker](wsl-docker.md), and live kernel probes, prepared fixtures and a release HTTP workflow pass. Live crash/recovery, active cancellation, timeout/resource-exhaustion and cross-job fault acceptance remain open. `docs/backend-engine-lock.json` retains `container_qualified: false`; the checked local image is not a qualified release.

## Execution boundary

The operator chooses a local rootless Docker Unix socket and a built image ID (`sha256:` plus 64 lowercase hexadecimal characters). Mutable tags, remote contexts and automatic pulls are rejected. Each readiness check verifies Linux x86_64, cgroups v2, rootless mode, seccomp, image labels and a real in-container probe. Each job probes its kernel restrictions again. Failure leaves queued jobs unclaimed; execution never falls back to a host subprocess.

The container runs as UID/GID 10001 with a read-only root, no network, no IPC sharing, all capabilities dropped, no-new-privileges, 2 CPUs, 2 GiB memory, zero swap, 128 PIDs and a 128 MiB per-file limit. `/work` is a 2 GiB tmpfs with `noexec,nosuid,nodev`. The probe verifies UID/capabilities/seccomp, root mount, network interfaces, cgroup limits, file limit and tmpfs size/options. A deployment without delegated rootless cgroups fails readiness.

The only host bind is a read-only copy of the receipt's ZIP and metadata. Its digest and byte count must match the durable submission. Host ancestors stay private. Extraction, configuration loading, path preflight and SBF execution happen inside the container. Extracted inputs have write permissions removed and their file hashes are checked before/after execution and case capture. This is a permission seal and mutation check; the extracted tmpfs tree is not a separate read-only kernel mount.

The helper calls the pinned CLI with server-selected arguments, preserves original report bytes, and captures eligible portable cases before the container disappears. Capture failure emits `PORTABLE_CASE_UNAVAILABLE` without changing FAIL into another verdict. Backend preflight errors complete with ERROR and a null engine exit/version. Actual CLI exit 4 becomes `failed/ENGINE_INTERNAL_ERROR`.

## Durable lifecycle

An exclusive OS file lock prevents two worker processes sharing a root. SQLite atomically claims one active job, persists a 30-second fenced lease and records coarse stages. Heartbeats run during staging/execution. Cancellation, timeout or shutdown removes the entire owned container before terminal acknowledgement. If removal cannot be confirmed, the job stays reserved for recovery and no result is published.

Restart recovery removes only containers bearing this storage root's namespace label, then resolves expired claims as `WORKER_LOST` or acknowledges cancellation. A still-live previous lease remains reserved until expiry. Queued jobs survive restart; failures require explicit new submissions. There is no automatic crash retry.

The output collector accepts fixed names and length frames, never host-selected container paths or container output archives. It checks ordering, invocation evidence, duplicate files, total bytes, EOF and report/exit agreement. Output is capped at 128 MiB, including the derived report, private original and retained diagnostics. Raw stderr is always drained with at most 8 MiB retained; partial logs remain private in failed staging. No raw logs enter owner-facing events. Fixed truncation/unavailable-case codes can appear in durable events.

Files are synced and atomically renamed from attempt staging to publication. Artifact digests and terminal result are committed together under the lease/cancellation fence. Unreferenced staging or renamed output cannot be downloaded through the store. The raw original and stderr stay private; the derived JSON has its own digest. HTTP downloads apply attachment/CSP handling to HTML.

Persistent storage rejects links/special files and inventories at most 100,000 entries. Before a claim, it requires 3 GiB free and at most 19 GiB already retained, reserving bounded staging beneath the proposed 20 GiB ceiling. This is a single-worker execution guard. B2 adds a serialized HTTP upload reservation and durable explicit deletion. Automatic retention/orphan cleanup and backup/restore are B4 work; trusted local operator submissions must keep storage within budget.

The worker also records successful runtime/capacity readiness in SQLite and refreshes it during execution/staging. The API requires a fresh observation (15-second TTL) and a held worker OS lock. A failed check invalidates readiness; an exited `check`/`once` process cannot leave the API ready solely through a retained database record.

Layout:

```text
STORAGE_ROOT/
  jobs.sqlite
  worker.lock
  docker-config/
  submissions/sub_<id>/archive.zip
  jobs/job_<id>/lease_<id>/
    input/                 temporary read-only bind source
    staging/               unpublished/failed evidence
    published/             atomically installed result, DB-authorized artifacts
```

## Local deployment and operator workflow

Use a dedicated Linux account with rootless Docker and delegated cgroups v2. Keep storage on a local filesystem; do not mount the Docker socket into jobs or expose these operator commands remotely. The configuration contains no bearer tokens. Copy `deploy/worker.example.json` and supply absolute paths and the actual local image ID.

Build the worker/helper, preserving the qualified engine binary (rebuilding the CLI requires a new engine qualification lock):

```bash
cargo build -p dg-service --release --locked --bins
python3 scripts/prepare-worker-image.py \
  --engine "$PWD/target/release/doppelganger" \
  --sandbox "$PWD/target/release/dg-sandbox" \
  --output "$PWD/report/worker-image-context"
```

The context copies these trusted binaries and their resolved local dynamic libraries into a scratch filesystem. `image-inputs.json` records their hashes. The output directory must be new. Build on the deployment Linux host; there is no remote base image or package installation. Only run this packaging script against trusted release executables because it uses `ldd`.

Build against the explicitly selected rootless socket, using the `ENGINE_SHA256` build argument printed by the script:

```bash
docker --host unix:///run/user/1000/docker.sock build --pull=false \
  --build-arg ENGINE_SHA256=81d88f39b08ee32879e899bde3300ff5bb80170b47fabbc2971792e863249e40 \
  --iidfile report/worker-image.id report/worker-image-context
```

Put the resulting image ID into the config; `dg-worker` supplies a private empty Docker config and explicit socket for every invocation. Create the storage parent first. The worker creates its root with mode 0700 or requires that permission on an existing root.

```bash
target/release/dg-worker /absolute/worker.json check
target/release/dg-worker /absolute/worker.json submit developer /absolute/metadata.json /absolute/project.zip
target/release/dg-worker /absolute/worker.json once
target/release/dg-worker /absolute/worker.json status developer job_<returned-id>
target/release/dg-worker /absolute/worker.json cancel developer job_<returned-id>
target/release/dg-worker /absolute/worker.json run
```

`submit`, `status` and `cancel` use owner-scoped store operations and may run while the worker owns the execution lock. They are trusted local operator tools, not a replacement for HTTP authentication. `run` serves the durable queue until Ctrl-C or SIGTERM. A running job is stopped/fenced before interruption exits the worker; an idle interruption exits immediately. Runtime/control reconciliation adds bounded cleanup time beyond the job deadline. Use restart recovery after abrupt process/host loss.

Prepared metadata and ZIP structure are defined in backend plan. `mode: source` remains unsupported.

## Acceptance and remaining gate

```bash
scripts/check-backend-worker.sh
DG_WORKER_CONFIG=/absolute/worker.json scripts/check-backend-worker.sh --container
```

The first command runs foundation, worker and API tests plus explicitly selected real-CLI fixture tests. It prints that live container checks were not run. The second additionally requires actual rootless Docker and runs API uploads/jobs for Vault/Escrow/Record PASS, broken Escrow FAIL with downloaded portable-case replay, and missing-artifact backend ERROR. It then builds a uniquely tagged fault image from a trusted test-only C program and tests active cancellation, timeout, PID/file/memory limits, contradictory output, storage exhaustion, hard-crash recovery and stale-publication fencing, cross-job isolation, queued-work resumption and graceful shutdown. Both fault and production-engine images remain digest-pinned to the same qualified engine; only temporary test containers/images are removed. Test storage is fresh and private beside the configured root; no existing deployment jobs are changed.

The runtime double exercises malformed/duplicate/truncated/oversized frames, verdict/exit mismatch, failed wrapper status, timeout, active cancellation/shutdown, retained/truncated logs, unavailable cases, exit 4, receipt mutation, exclusive ownership, orphan recovery, failed removal, storage budget/links and a version-1 database upgrade preserving queued events/idempotency. Existing foundation tests cover stale publication and atomic DB transactions. These are local tests, not live kernel qualification.

The complete live rootless Docker command now passes on this host. Runtime probes, prepared fixtures and worker fault cases confirm the container boundaries and durable recovery paths. Repeat this command on every deployment host before release; kernel/runtime behavior is host-specific. The B1/B2 functional gate is closed for this host. B3 source builds and B4 release, load, retention and backup acceptance remain open.

Recorded local checks on 2026-10-08: **457 workspace tests passed, zero failed, four gated tests ignored**. Two trusted engine-contract tests then passed explicitly, for 459 distinct tests executed. Thirty service foundation/worker tests pass; the live container test and Devnet RPC test remain unexecuted. Formatting and workspace/all-target Clippy with warnings denied passed. Cargo-audit reports zero known vulnerabilities and five existing upstream maintenance notices. The lockfile adds only `signal-hook-registry`; existing package versions are unchanged.

Both release executables built successfully. The scratch image context contains six runtime files with verified recorded hashes; packaging rejects a changed engine before creating a context. The actual worker `check` command returned `RUNTIME_UNAVAILABLE` for the missing local Docker socket. Release operator commands submitted a packaged Vault ZIP, read its queued state and cancelled it without execution.

Evidence from the original B1 checkpoint: `report/backend-worker.4halD4/acceptance.log`, `report/backend-worker-workspace.log`, `report/backend-worker-clippy.log`, `report/backend-worker-audit.json`, `report/worker-image-context-20261008/image-inputs.json` and `report/backend-worker-operator-check/operator-evidence.json`. These generated/private files remain ignored. At that checkpoint the context was assembled but the OCI image had not been built or run. Subsequent native runtime setup and live acceptance are recorded in [WSL Docker verification](wsl-docker.md).
