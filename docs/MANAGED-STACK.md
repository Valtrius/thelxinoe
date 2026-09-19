# Managed services

Administrators can install Radarr, Sonarr, Lidarr, Bazarr, Prowlarr and NZBGet from Settings. The controller accepts only curated templates and immutable image identities. Appdata lives below its persistent deployment directory; created services share the server's physical `/data` mount. Advanced UI ports bind to host loopback. An optional administrator-supplied URL can point to a separately configured reverse proxy.

Installation records an encrypted credential and durable job before submitting Docker work. The controller journals container creation before issuing it. Interrupted or uncertain mutations require reconciliation; retries never silently accept configuration drift. Installed services connect to NZBGet, Prowlarr and Bazarr automatically. Choose acquisition profiles and canonical library folders in manager settings, then configure indexers, subtitle providers and a news server using the services' own advanced settings.

Adoption starts with an existing API integration. It accepts compatible standalone containers using the tested image and supported mounts/runtime configuration on the server network. Compose, Swarm, Kubernetes, Podman Compose and Nomad ownership labels block adoption. The original stops and is retained during replacement; appdata and the integration ID remain unchanged. The original is retired only after the replacement API connection succeeds. Custom commands, privileged devices and unsupported network configuration are rejected.

Managed configuration is fingerprinted separately from runtime addresses. External changes block lifecycle mutations. Reconciliation can complete an interrupted recorded operation, but cannot bless arbitrary new configuration.

## Deployment recovery

The controller stores the accepted first-party image identities, original inspect specifications and generation outside server SQLite. Each immutable generation contains both the descriptor and a complete Compose override. An atomic filesystem pointer advances both files together. The generated deployment directory contains a base `compose.yaml` and `compose.override.yaml`; run Compose there for recovery. The override takes precedence over stale bootstrap image tags. Accepted images must still be present locally for offline recovery.

This directory contains private configuration and must remain administrator-only. It requires a Linux filesystem with atomic rename and symlink support. Docker Desktop test fixtures use a named Linux volume; a Windows shared directory is unsuitable for the generation pointer. The generated Compose uses the Docker host's resolved mount paths and external media network. Preserve those paths, the runtime volume and the media network when recovering. Development rebuilds of the bootstrap images do not advance the accepted release generation.

Managed-service updates require [isolated compatibility checks and recovery snapshots](SERVICE-UPDATES.md). First-party release handoff is a subsequent roadmap phase.

## Verified fixtures

`compose.managed.test.yaml` creates an isolated deployment at `https://localhost:24443`. On fresh fixture state:

```powershell
docker compose -f compose.managed.test.yaml up -d --wait
node scripts/test-managed-stack.mjs
node scripts/test-managed-wiring.mjs
node scripts/test-managed-ui.mjs
node scripts/test-deployment-compose.mjs
```

The installation proof creates all six services, validates their real API connections, and records only sanitized IDs. Wiring checks inspect all three download clients, Prowlarr's three applications and Bazarr's manager connections. The browser proof saves canonical roots and demonstrates that changing a test container's restart policy blocks lifecycle actions, then restores that policy. The Compose proof verifies both accepted image pins and precedence over a stale bootstrap file.

`compose.adoption.test.yaml` and `scripts/test-managed-adoption.mjs` use another isolated deployment on HTTPS port 25443. The test rejects foreign Compose ownership, adopts standalone Radarr, verifies a retained appdata sentinel and integration identity, and verifies retirement of the original. It intentionally refuses to recreate a previously used fixture. A blocked setup can be retried through the authenticated first-party retry action after correcting the cause.

Sanitized local results: `.local/managed-install-result.json`, `.local/managed-wiring-result.json`, `.local/managed-ui-result.json`, `.local/adoption-result.json`. Unit tests cover privilege boundaries, encrypted job credentials, foreign ownership, configuration drift and atomic generation commits. Linux controller/server tests and Windows production installer builds are part of the validation gates.
