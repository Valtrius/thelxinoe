# Managed service update safety

Settings provides a server default and per-service Automatic, Notify or Manual policy. Notify is the default. Services may inherit the server policy. Maintenance windows use explicit UTC hours and may cross midnight. Automatic checks run every six hours; automatic installation waits for the window, idle service APIs and no active playback. Administrators can request a compatibility check or install a verified candidate directly.

Candidates resolve only from each curated repository's stable channel to an immutable digest. No public endpoint accepts an arbitrary image, mount, executable or Docker command. A durable server job and independent controller journal record each attempt. Concurrent submissions create one pending attempt. Missing permissions, drift, unresolved interruptions and unsupported appdata block progress.

## Consistency and isolation

Each curated service uses stop-and-copy appdata snapshots. The controller verifies the managed container, stops it, checks that it stopped, and launches a trusted disposable copy worker. The source is mounted read-only, and the destination is a newly allocated directory in deployment state. Regular files and directories are supported; symbolic links, hard links, devices, oversized data and excessive file counts fail verification. Copies preserve ownership and permissions and sync their contents. Appdata must be separate from media and system directories. Snapshots are private recovery material, not portable encrypted backups.

The preflight candidate receives a disposable appdata clone and disposable scratch data. Its network contains only loopback. It has no production network, host port, Docker/controller socket, real media mount or arbitrary network route. CPU, memory and process limits apply. The trusted contract runner shares that network namespace and verifies both the interface list and failed connections to forbidden endpoints.

The contract checks authenticated APIs and expected response shapes for all six services. Manager download-client tests use a controlled local NZBGet stub. Bazarr's version queries use local Radarr/Sonarr stubs; every other request to those peers returns an error so background sync cannot treat an empty fixture as the real library. Supported dependency hostnames resolve to loopback during validation. Configurations that cannot be verified within this isolation remain blocked.

## Activation and recovery

After preflight succeeds, installation takes another consistent recovery snapshot. The candidate starts against the actual appdata while still isolated, allowing migrations and startup validation without production side effects. Failure here removes the candidate, restores the recovery copy, and restarts the prior container. The test suite explicitly mutates appdata before causing candidate failure and checks that those mutations disappear after restoration.

The journal records the production activation boundary before starting the replacement with its normal networks and mounts. The accepted container retains the managed service and API integration identities. Authenticated health checks run after activation. Failures after this boundary are reported as runtime failures; automatic rollback is prohibited because restoring appdata cannot reverse production side effects. The prior stopped container and recovery snapshot remain available for an explicitly planned restore.

Interrupted pre-activation work requires the recovery action. Recovery rejects an obsolete attempt after a newer service has been accepted. Helper images receive retained references so replacing a bootstrap tag does not remove the image required by an ongoing snapshot or contract run.

## Local evidence

The isolated managed fixture has passed preflight for Radarr, Sonarr, Lidarr, Bazarr, Prowlarr and NZBGet. Bazarr initially failed without dependency stubs, demonstrating that checks remain blocked when required behavior cannot be verified safely. The corrected contract passed without adding production egress.

The real Docker tests exercised replacement using the currently published stable digest, retained integration identity, a healthy API after activation, and restoration after a simulated migration wrote appdata and the candidate was stopped. They do not claim validation of an unpublished upstream version or its future migrations. Each new candidate must pass its own runtime checks.

```powershell
node scripts/test-service-preflights.mjs
node scripts/test-service-update-rollback.mjs
node scripts/test-service-update-commit.mjs
```

Use the managed test fixture described in [managed stack setup](MANAGED-STACK.md). The rollback test consumes the recorded successful Radarr preflight; run the preflight test again before repeating it. A kind argument, such as `bazarr`, reruns only that service's check.

Sanitized evidence is in `.local/service-preflights.json`, `.local/service-rollback-result.json`, `.local/service-update-result.json` and `.local/service-updates.png`. Unit tests cover concurrent submissions, administrator permissions, maintenance windows, snapshot entry restrictions and rejection of rollback after activation. The Windows installer, frontend checks and Linux controller/server tests are included in the build gates.

## Managed-service updates

Each service has Automatic, Notify, or Manual update policy, with a server default and per-service override.

An update must use a stable upstream release and pass compatibility preflight.

Every curated service defines a consistent appdata snapshot strategy. The strategy may use a service-supported backup/export, a filesystem snapshot with defined consistency guarantees, or a short quiesce/stop-and-copy operation. A recursive copy of a live database-bearing appdata directory is not considered a valid snapshot merely because the files can be read.

Preflight flow:

1. Resolve and pull the candidate immutable digest.
2. Create a consistent disposable appdata snapshot using the service's snapshot strategy.
3. Sanitize or override runtime wiring that would point at live peers, then start the candidate against the disposable snapshot on a dedicated preflight network.
4. Give the candidate no production Docker networks, no Docker socket/controller access, no host-service access, and no arbitrary outbound egress. Mount media read-only when the contract needs media visibility. Publish no host ports.
5. Provide only controlled disposable/stub dependencies needed by the adapter contract. The test runner can reach the candidate on the preflight network; the candidate cannot reach live media-stack instances or other production endpoints.
6. Let candidate migrations run on the disposable snapshot.
7. Run adapter contract and smoke tests.
8. Classify the candidate as compatible, incompatible, or unable to verify. If required behavior cannot be tested without unsafe production access, classification is unable to verify rather than weakening isolation.

Only compatible candidates can install automatically.

Before live replacement, Thelxinoe waits for the service to be idle, takes another consistent recovery snapshot, and records the exact old image/spec. The old container is then stopped.

The candidate first starts against the real appdata inside the same isolation boundary used for preflight. Migrations, startup, health checks, and the adapter contract run before the candidate receives production networks or normal access to peer services. If this isolated live-state validation fails, Thelxinoe stops the candidate, restores the recovery snapshot and prior image/spec, and restarts the old service. Because the candidate had not crossed the production activation boundary, this rollback does not need to undo API mutations in live peers.

After isolated validation succeeds, Thelxinoe starts the accepted image with its production networks, mounts, and wiring and marks the update committed. Once a service has crossed that activation boundary and can perform normal external work, later failures are runtime failures rather than assumed side-effect-free update rollbacks. An explicit recovery restore may still be offered, but restoring appdata is not claimed to reverse changes already made to media, downloads, or peer services.

Automatic updates wait for the affected service to be idle and can use an admin maintenance window.
