# Managed services

Administrators can install Radarr, Sonarr, Lidarr, Bazarr, Prowlarr and NZBGet from Settings. The controller accepts only curated templates and immutable image identities. Appdata lives below its persistent deployment directory; created services use the server's single writable `/media` bind mount. Advanced UI ports bind to host loopback. An optional administrator-supplied URL can point to a separately configured reverse proxy.

Installation records an encrypted credential and durable job before submitting Docker work. The controller journals container creation before issuing it. Interrupted or uncertain mutations require reconciliation; retries never silently accept configuration drift. Every service installs independently. Choose acquisition profiles and canonical library folders in manager settings, then configure indexers, subtitle providers and a news server using the services' own advanced settings.

Ownership transfer starts with an existing API integration. It copies the service's appdata into Thelxinoe's deployment storage and creates a managed replacement using the exact installed stable LinuxServer image. Application settings, API credentials, library records and the integration ID are preserved. The original container remains stopped with automatic restart disabled; its config folder is retained. Custom commands, privileged devices, cluster ownership and unsupported mounts or network configuration are rejected.

## Taking ownership of an existing service

1. Connect the existing service first, following the [shared media and network requirements](STORAGE.md). Transfer requires one shared Docker network, writable bind mounts at `/config` and `/media` (only `/config` for Prowlarr), and the standard internal service port. Appdata must be outside both media and Thelxinoe's deployment directory. Symlinks, hard-linked appdata files and special files are not supported by the configuration copier.
2. Open **Settings → Media services → Managed services**, choose the connected service, and click **Review ownership transfer**. The review shows the current container, retained image, source config directory and managed destination. It does not stop the service.
3. For a Compose service, remove its definition from the original Compose file or assign a profile that you leave disabled. Save that edit without redeploying yet, and confirm it in the review. Disable external updaters or recreation scripts for the service too. Thelxinoe cannot edit your NAS Compose files or prevent you from explicitly starting the old service later.
4. Click **Stop, copy and take ownership**. Thelxinoe checks the reviewed Docker configuration again, checks that media-manager activity is idle, disables the original container's restart policy, stops it and copies its config into `deployment/services/<installation-id>/appdata`. The replacement keeps its image version, UID/GID, timezone, API credentials, published ports and network aliases. Application settings are not regenerated or automatically rewired.
5. After the replacement's API connection succeeds, its lifecycle, backups and update policy become available through Thelxinoe. No application version update is performed during the transfer. Subsequent updates use the normal isolated compatibility checks.

Do not restart the retained original alongside the managed replacement: both still use the shared media directory. Avoid Compose `--remove-orphans` while you want to retain the original container as a recovery aid. After verifying the managed service, you can remove the old stopped container yourself and archive its original config.

An interrupted transfer retains its journal and copied data. **Retry setup** retries API connection; **reconcile** verifies a recorded replacement after an interrupted Docker operation. If copying failed or the replacement cannot be accepted, **Restore original service** removes only the transfer's replacement/workers, restores the original restart policy and running state, and reconnects the original integration. The copied config remains on disk for inspection; changes made there are not merged back. Once transfer completes, use managed backups and update recovery instead.

Managed configuration is fingerprinted separately from runtime addresses. External changes block lifecycle mutations. Reconciliation can complete an interrupted recorded operation, but cannot bless arbitrary new configuration.

The service view refreshes runtime status while open. A stopped container, a missing
container, an unavailable inspection and changed Docker configuration are reported
separately. **Reconcile** also reconnects the server's API integration to an accepted
replacement container and queues API verification while preserving the integration ID.

For a missing container, **Recreate and start** uses the recorded image/specification
and preserved appdata. Interrupted creation can be resumed with **Retry setup** or
**Reconcile**; the controller checks all container ownership labels before creating
anything, including renamed containers and ambiguous earlier create results. Missing
appdata, conflicting ownership and Docker inspection failures block recreation.

**Retire missing installation** releases an orphaned installation and its active API
connection without deleting appdata or media. Request history and old file ownership
evidence are retained, and unfinished requests are cancelled. A failed retirement can
be retried; the controller retains its journal and appdata after releasing ownership.

See [storage and external NAS services](STORAGE.md) for Compose examples, `/media/tv`, and permissions.

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

`compose.adoption.test.yaml` and `scripts/test-managed-adoption.mjs` use an isolated deployment on HTTPS port 25443. The script creates a fresh project and storage directory, and cleans up its containers, network and volumes in `finally`:

```sh
docker build --target server -t thelxinoe-takeover-server:local .
docker build --target controller -t thelxinoe-takeover-controller:local .
node scripts/test-managed-adoption.mjs
```

The proof transfers a real Compose-managed Radarr through the UI. It rejects missing ownership confirmation and stale reviews, forces a copy failure and restores the original after a controller restart, then verifies separate managed appdata, retained settings/version/permissions/ports, the integration identity, managed restart and a real isolated update preflight. Results and browser screenshots remain in `.local/ownership-test-<run>/`.

Other sanitized local results: `.local/managed-install-result.json`, `.local/managed-wiring-result.json`, `.local/managed-ui-result.json`. Unit tests cover privilege boundaries, encrypted job credentials, foreign ownership, configuration drift and atomic generation commits. Linux controller/server tests and Windows production installer builds are part of the validation gates.

## Managed Docker stack

V1 can install or adopt local Docker instances of Radarr, Sonarr, Lidarr, Bazarr, Prowlarr, and NZBGet. Remote API-only instances are deferred.

Thelxinoe manages stable upstream releases only. Nightly, develop, beta, preview, and similar channels are permanently outside the managed-service model.

Thelxinoe-created services use curated images and templates. Installed versions are stored by immutable image digest.

The controller owns Docker-level configuration such as image, mounts, networks, ports, environment, restart policy, labels, and lifecycle. Each service owns its application-level configuration.

Docker-level ownership is exclusive. The administrator releases the previous Compose definition before transfer; the replacement carries only its new Thelxinoe ownership. Swarm, Kubernetes, Podman Compose, Nomad and another Thelxinoe deployment remain unsupported transfer sources. The controller records the supported configuration and immutable image before taking ownership. Drift detection reports outside mutations instead of accepting a second owner.

Cross-service connections are optional and require **Connect** in the source service's settings: **Applications** under Prowlarr, **Download clients** under Radarr/Sonarr/Lidarr, and **Media managers** under Bazarr. A compatible registered service appears as **Available to connect**. Leaving it disconnected does not affect either service's installation or health. API integrations can use these connections with or without Docker ownership.

Enabled connections are persisted and retry independently after outages, restarts, address changes and container replacement. Only the connection shows an unavailable state when its source or target is stopped. **Disconnect** immediately disables reconnection; removal of configuration created by Thelxinoe finishes when the source is available. An upstream record's saved ID identifies it even after renaming. Changes to fields controlled by the connection require attention instead of being overwritten. Existing manual connections are preserved. NZBGet categories use unique names and free slots; disconnecting retains categories so existing downloads keep their routing.

Servarr APIs mask saved keys and passwords, so their exact values cannot be compared for outside edits. Thelxinoe preserves those saved secrets during address updates and uses the service's connection test to check them. After changing a service credential, update the upstream connection in its advanced settings, or disconnect and connect it again.

## Container names

The bootstrap services are named `thelxinoe-server` and `thelxinoe-controller`. New and adopted managed services use `thelxinoe-radarr`, `thelxinoe-sonarr`, `thelxinoe-lidarr`, `thelxinoe-bazarr`, `thelxinoe-prowlarr` and `thelxinoe-nzbget`. If another deployment already owns the preferred name, the controller adds a short installation suffix. Adoption preserves the original configuration for recovery and recreates the accepted service under its Thelxinoe name. Existing prefixed installations retain their recorded names through updates and restore.
