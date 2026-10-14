# Native rootless Docker on this WSL host

Installed and checked on 2026-10-09 for Ubuntu 26.04.1 (`Ubuntu` WSL distribution), Linux x86_64 kernel `6.6.87.2-microsoft-standard-WSL2`, user `kuro` (UID 1000).

The native engine uses [Docker's official Ubuntu apt repository](https://docs.docker.com/engine/install/ubuntu/) and [rootless setup](https://docs.docker.com/engine/security/rootless/). The previous `/usr/bin/docker` link pointed to unavailable Docker Desktop WSL integration; it is now the package-managed native CLI.

## Installed configuration

| Component | Observed version/configuration |
| --- | --- |
| Docker Engine / CLI / rootless extras | 29.9.0 |
| containerd | 2.4.1 |
| Buildx | 0.38.0 |
| Compose | 5.6.0 |
| Rootless networking | slirp4netns 1.3.3 |
| Context / socket | `rootless` / `unix:///run/user/1000/docker.sock` |
| Data directory | `/home/kuro/.local/share/docker` |
| Cgroups | v2, systemd driver; CPU, memory and PID delegation |
| Daemon security options | rootless, seccomp builtin, cgroup namespace |

Installed `uidmap` and user-session prerequisites. Existing subordinate UID/GID ranges (`kuro:100000:65536`) were retained. The system-wide Docker service/socket are disabled; the enabled user `docker.service` owns the rootless daemon. User lingering is enabled so the user service starts with the WSL distribution and survives shell logout. Starting WSL itself on Windows boot is outside this setup.

The package setup script inserted the inherited Windows PATH into its systemd unit without quoting spaces. Its PATH assignment was replaced with a quoted Linux executable search path, then the user daemon was reloaded/restarted and checked again. No global namespace, seccomp or cgroup restrictions were disabled.

## Everyday commands

```bash
docker context show
docker info
docker run --rm hello-world
docker compose version
systemctl --user status docker --no-pager
```

Control the installed daemon with `systemctl --user start|stop|restart docker`. Use the native CLI/socket explicitly when another tool overrides Docker context:

```bash
/usr/bin/docker --host unix:///run/user/1000/docker.sock info
```

The worker already selects this explicit local socket and its own private Docker client configuration. Its configuration is saved at `/home/kuro/doppleganger/report/docker-wsl-install/worker.json`; it points to separate private test/operator storage. The API and worker used for acceptance were shut down normally. Only Docker remains running.

## Backend verification

Built a local scratch worker image from the unchanged qualified engine and current release sandbox helper. Its observed image ID is `sha256:a4bf08d7e6844a414c85bcfbc184628d695b4d82a914d28ac243704bcd8a219d`. It is a host-local image, not a published release. The context and input hashes are saved under `report/worker-image-context-20261009`.

Verified:

- Default rootless context runs `hello-world`; the daemon remains usable after restart.
- Release `dg-worker ... check` returns `ready: true` after the actual in-container probe checks non-root identity, capabilities, seccomp, no-new-privileges, read-only root, network denial, CPU/memory/swap/PID/file limits and bounded tmpfs.
- `scripts/check-backend-worker.sh --container` passes: 41 default service tests, two trusted engine-contract tests and the live container fixture test (44 tests executed). The live test uploads Vault/Escrow/Record through API handlers, checks PASS, checks broken Escrow FAIL and downloaded case replay, and checks missing-artifact ERROR.
- A separate release-process test starts `dg-api` and `dg-worker run`, uploads a prepared Vault ZIP from outside the repository through the smoke client, observes readiness 200 and completed PASS, and verifies all three downloaded artifact hashes/sizes. Both processes then exit normally on SIGTERM; no test containers remain.
- `scripts/check-backend-worker.sh --container` also passes live faults in a temporary, uniquely tagged image built from trusted test-only C code: active cancellation, timeout, PID and file limits, simulated contradictory engine output, persistent-storage rejection, kernel OOM, worker hard-crash recovery after lease expiry, stale publication fencing, continued queued execution, cross-job path isolation and graceful SIGTERM cleanup. It removes the test image and its base tag afterward.

During acceptance, the lock-contention test exposed a timing assumption when another test forks a subprocess. A file lock can briefly survive its parent's close through the child's inherited descriptor until exec. A small host reproduction verified this; the test now bounds its setup wait at two seconds before asserting deliberate contention. Production reservation logic is unchanged.

**Remaining qualification:** this host passes live prepared execution and worker-fault acceptance; repeat the gate on other deployment hosts before release because kernel/runtime behavior is host-specific. The installed Docker image is local and is not a qualified release artifact, so `backend-engine-lock.json` correctly retains `container_qualified: false`. Source-folder builds remain B3.

## Evidence

Generated evidence remains ignored under `report/docker-wsl-install/`: package/setup logs, package versions, Docker info, original user unit, hello-world output, image identity/build logs, release readiness output, live fixture logs, and `live-http-evidence.json` plus downloaded output. The successful shared gate also retained `report/backend-worker.ifkBxR/acceptance.log`. Docker client credentials are not included in the report.
