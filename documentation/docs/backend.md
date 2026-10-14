# Prepared-project API

The implemented HTTP API exposes prepared uploads, durable jobs and allowlisted reports. It is separate from local CLI usage and this static documentation site.

## Operator architecture

`dg-api` and `dg-worker` run as separate processes on one qualified Linux host with the same absolute private storage root. SQLite and local OS locks coordinate ownership, queue state and readiness. The worker executes uploaded parsing and SBF testing inside rootless Docker containers; the API never executes uploads itself.

Build the service binaries from the repository root:

```bash
cargo build --release -p dg-service --bins --locked
```

Provision actual private configs based on `deploy/api.example.json` and `deploy/worker.example.json`. Placeholder hashes/image IDs are intentionally invalid. `dg-api /absolute/api.json` and `dg-worker /absolute/worker.json run` require that provisioning. They are not needed for the local tutorial.

## Implemented routes

Liveness/readiness, authenticated capabilities, ZIP submission receipt/deletion, owner-scoped job creation/list/poll/cancel/delete, durable event polling, derived report retrieval and verified allowlisted artifact downloads are implemented. Bearer tokens map to operator-selected owners; browser access requires explicit CORS origins. Do not embed an operator bearer token into a public frontend bundle.

The current mode is `prepared`; raw Rust source-folder mode is unsupported. A receipt confirms upload reception, not project validity. New jobs return 503 without a ready worker.

## Sandbox and storage requirements

The worker requires a local rootless Docker Unix socket, Linux x86_64, cgroups v2, seccomp and delegated resource limits. Each job has no network, a read-only root, dropped capabilities, a bounded tmpfs, 2 CPUs and 2 GiB memory limits. Readiness probes verify restrictions before claiming jobs.

Persistent local storage retains SQLite, uploads and evidence. Supabase/object storage is not a drop-in mount or SQLite replacement; using it requires changes to intake, publication, queue/lease coordination and recovery. Public hosting and load/retention/backup acceptance are separate work.

## Maintainer references

The repository's current service documentation is in `docs/backend-api.md`, `docs/backend-worker.md`, `docs/backend-foundation.md` and the versioned contract `docs/backend-api.yaml`. Those files are included in the source checkout; public GitHub links are not presented as usable until source publication.

No backend integration is required to host this documentation site. See [known limitations](/docs/limitations) for unfinished product features.
