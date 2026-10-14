# Prepared-project API checkpoint

The B2 HTTP implementation is available as `dg-api`. It shares the durable SQLite queue and private storage with `dg-worker`. Uploads, owner-scoped jobs/events, cancellation, report retrieval and verified downloads can now be integrated with a prepared-project frontend.

**Live prepared execution now passes on this WSL host.** [Native rootless Docker](wsl-docker.md) passes the isolation probe, prepared fixture gate and a real release HTTP upload/job/download workflow. The API still returns HTTP 503 for new jobs without a ready worker. Full live fault qualification remains open, so the engine lock still records `container_qualified: false`. Source-folder compilation is B3 and is not enabled.

## Implemented workflow

| Route | Behavior |
| --- | --- |
| `GET /health/live` | Process liveness, no token required |
| `GET /health/ready` | 200 only while worker ownership, fresh readiness and storage capacity agree; otherwise 503 |
| `GET /v1/capabilities` | Supported modes and enforced limits |
| `POST /v1/submissions` | Stream one ZIP plus JSON metadata; return immutable receipt with SHA-256 and byte count |
| `GET /v1/submissions/{id}` | Retrieve the authenticated owner's receipt |
| `DELETE /v1/submissions/{id}` | Delete an unreferenced submission with durable filesystem cleanup |
| `POST /v1/jobs` | Enqueue a prepared submission with optional `Idempotency-Key` |
| `GET /v1/jobs` | List the owner's jobs, at most 100 per page |
| `GET /v1/jobs/{id}` | Poll state, verdict, errors and public artifact inventory |
| `POST /v1/jobs/{id}/cancel` | Request cancellation; poll until acknowledgement |
| `DELETE /v1/jobs/{id}` | Delete a terminal job and its artifacts; active jobs return 409 |
| `GET /v1/jobs/{id}/events` | Durable events, at most 100 per page, integer `after` cursor |
| `GET /v1/jobs/{id}/report` | Derived JSON report for a completed result |
| `GET /v1/jobs/{id}/artifacts/{kind}` | Download an allowlisted, size/hash-verified published artifact |

The versioned contract is [backend-api.yaml](backend-api.yaml); upload structure and engine state/verdict mapping are in backend-plan.md. A receipt confirms reception, not ZIP or project validity. The API never extracts an upload, compiles contracts, executes SBF or controls Docker. All uploaded project parsing stays inside the worker sandbox.

Bearer tokens map to fixed owners chosen by the operator. Token digests are compared in constant time. Unknown or foreign resource IDs return 404. Errors use the contract envelope and carry the same request ID as `X-Request-ID`; authentication errors return 401. Explicit CORS origins are required for browser requests. Responses disable caching and MIME sniffing.

Artifacts are limited to `report_json`, `report_html`, `summary`, and optional `portable_case`. Storage paths, the original private report and stderr are not exposed. Downloads use fixed attachment filenames, SHA-256 ETags and a sandbox CSP. Artifact readers and response bodies each hold a bounded concurrency permit; a slow client continues to occupy a download slot until it finishes or disconnects.

## Local deployment

Use one dedicated Linux account and a local filesystem. Run the API and worker as separate processes with **the same absolute `storage_root`**. Private directories require mode 0700; API configuration must be a regular file without group/world permissions. A second API or worker instance for the same root fails its respective OS lock.

Build the service executables without replacing the qualified engine:

```bash
cargo build --release -p dg-service --bins --locked
```

Provision a random token and its hashed API configuration once. This example saves the token to a private operator file rather than printing it:

```bash
python3 - <<'PY'
import hashlib, json, os, secrets
from pathlib import Path
directory = Path.home() / '.config' / 'doppelganger'
directory.mkdir(mode=0o700, parents=True, exist_ok=True)
config = json.loads(Path('deploy/api.example.json').read_text())
token = secrets.token_urlsafe(48)
config['tokens'][0]['sha256'] = hashlib.sha256(token.encode()).hexdigest()
for name, content in [('api.token', token + '\n'), ('api.json', json.dumps(config, indent=2) + '\n')]:
    with os.fdopen(os.open(directory / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as file:
        file.write(content)
PY
```

Edit that private `api.json` to select storage and exact frontend origins. The default bind is `127.0.0.1:8787`; only loopback addresses are accepted. The example placeholder digest in `deploy/api.example.json` deliberately fails validation until provisioned. Tokens must contain 32–4096 printable ASCII characters. Multiple digests may map to one owner for rotation; duplicate digests are rejected. Config changes take effect after an API restart, while durable jobs continue in the worker.

```bash
target/release/dg-api "$HOME/.config/doppelganger/api.json"
```

Provision and qualify the rootless runtime using [backend-worker.md](backend-worker.md), then run `dg-worker /absolute/worker.json run` in a separate terminal. API uploads remain available while the worker is unavailable. New job creation requires a worker lock plus a successful readiness observation newer than 15 seconds; a standalone `check` command does not leave the API ready after it exits. A valid idempotent retry returns its original job even when the worker subsequently becomes unavailable.

For access from another machine, terminate TLS at an authenticated deployment ingress forwarding to the loopback API. Keep that ingress private and configure connection, header and response-write timeouts; the API's 120-second maximum deadline covers receiving/handling a request, not streaming the completed download to a slow reader. Provision tokens outside frontend source code and avoid credentials in URLs or logs.

## Smoke client and frontend integration

Create a ZIP whose relative structure matches the metadata, for example:

```text
project/
  doppelganger.toml
  doppelganger-policy.toml
  v1.bin
  v2.bin
  v1.json
  v2.json
```

`metadata.json` selects `project/doppelganger.toml` and `project/doppelganger-policy.toml`, with `schema_version: 1`, `mode: "prepared"`, and `snapshot_path: null` unless explicitly supplying a frozen snapshot. Manifest-relative binaries/IDLs must resolve inside the ZIP. A raw folder of Rust sources is not a prepared project.

```bash
DG_API_TOKEN="$(cat "$HOME/.config/doppelganger/api.token")" \
  scripts/smoke-backend-api.py \
  --metadata /absolute/metadata.json --archive /absolute/project.zip \
  --output /absolute/new-smoke-directory
```

The client streams uploads, checks receipt integrity, submits/polls a job, saves events/report and verifies downloaded artifact bytes/hashes. The output directory must be new. A completed FAIL or input ERROR is a legitimate engine outcome; worker failure/cancellation or transport/integrity failure exits unsuccessfully. Add `--upload-only` to validate reception while execution is unavailable. The client never extracts or replays an uploaded portable case on the host.

For the frontend:

1. Read authenticated capabilities and health; distinguish upload availability from execution readiness.
2. Package selected folder files into one ZIP, preserving relative paths. Send exactly two multipart parts named `metadata` and `archive`. Let the browser set the multipart boundary.
3. Save the receipt before creating a job. Use a stable `Idempotency-Key` for retries of that creation request; it contains 1–128 printable ASCII characters. Changing the submission under the same key returns 409.
4. Poll jobs roughly every two seconds and advance event `after` using `next_cursor`. Jobs use an opaque job ID as the list cursor. Persist IDs across reloads; coarse durable stages are not a percentage complete.
5. Show job state separately from verdict. `completed` can mean PASS, FAIL or input ERROR. Only completed results expose report/download routes. Cancellation is acknowledged through polling.
6. Offer downloads from the returned artifact inventory. Retrieve them with the bearer header and save a browser blob; do not place the token in a download URL. Display an unavailable portable case as unavailable evidence.
7. Honor HTTP 429 and `Retry-After`. Show actionable errors and request IDs. Delete terminal jobs before deleting their referenced submission.

## Limits and remaining work

The API enforces a 384 MiB archive cap, 32 KiB metadata cap, one active upload, 64 submissions per owner, four queued jobs per owner, 16 queued jobs globally, 120 authenticated requests per owner per minute, 64 concurrent handlers and two active report/download responses. Actual chunk bytes are counted. Partial, timed-out and normally disconnected uploads remove their temporary receipt directories.

Upload reservation holds an OS lock and reserves the maximum archive beneath a 19 GiB retained-storage threshold, requiring 3 GiB free plus the reservation. This leaves a margin under the proposed 20 GiB deployment ceiling for the single worker's bounded staging. Capacity failure returns 503; contention/quota failure returns 429. Trusted local operator submission commands must respect the same budgets; they do not acquire the HTTP upload reservation.

Submissions become ineligible for **new** jobs after seven days. Automatic retention is not implemented: existing records/files remain until explicitly deleted. Deletes commit metadata removal and a cleanup intent together; filesystem removal is retried after API restart until all pending pages are drained. A hard crash during receipt publication/registration can leave an unregistered file; automatic orphan cleanup, scheduled retention and backup/restore qualification belong to B4. No public-hostile-source or load qualification is claimed.

## Acceptance evidence

Run the existing shared gate; it now includes API tests and OpenAPI envelope checks:

```bash
scripts/check-backend-worker.sh
DG_WORKER_CONFIG=/absolute/worker.json scripts/check-backend-worker.sh --container
```

Local checks cover owner isolation, CORS/authentication, request IDs, bounded/partial uploads, request deadlines, idempotency, queue/storage/rate limits, expiry, terminal deletion, paginated cleanup recovery, report/download integrity, download-slot lifetime and a real TCP smoke-client round trip with a controlled runtime. Existing intake tests reject malformed/traversing/linked/duplicate/oversized ZIPs inside the extraction boundary.

The optional container gate uploads the trusted Vault/Escrow/Record fixtures through API handlers and downloads/replays the broken Escrow case. It now passes against [native rootless Docker](wsl-docker.md), along with an external-folder release HTTP smoke test. These live checks replace the previous unavailable-runtime blocker. Full source-folder drag-and-drop remains dependent on B3 builds and remaining live backend fault acceptance.

Recorded local checks on 2026-10-09: **468 workspace tests passed, zero failed, four gated tests ignored**. The two trusted engine-contract tests also passed explicitly, for 470 distinct tests executed. The service gate passed 41 default tests (19 foundation, 11 worker, 11 API) plus those two engine-contract tests. Formatting and workspace/all-target Clippy with warnings denied passed; the final service changes also passed strict Clippy. Cargo-audit reports zero known vulnerabilities and five existing upstream maintenance notices. The lockfile adds 11 HTTP-related packages and changes no existing package versions.

The release service binaries built successfully. The actual release API accepted a packaged Vault upload, verified its receipt, retained it across restart, rejected foreign-owner access, and removed it through durable deletion. Without a worker, readiness and new job creation both returned 503. These checks did not execute a container. The qualified engine's recorded SHA-256 remains unchanged.

Ignored evidence: `report/backend-worker.cAQT1x/acceptance.log`, `report/backend-api-workspace.log`, `report/backend-api-clippy.log`, `report/backend-api-final-clippy.log`, `report/backend-api-audit.json`, `report/backend-api-release-build.log`, and `report/backend-api-operator-check/operator-evidence.json`. The latter records both the checked API binary digest and engine digest. Generated uploads, private operator configuration and release binaries remain outside version control.
