# Managed services and retention acceptance

These are the accepted v1 delivery criteria. Current results and supported limits are in [implementation status](../STATUS.md) and the [release checklist](../RELEASE_CHECKLIST.md).

## Phase 11. Requests and media-manager API integrations

Add API-level acquisition before Docker lifecycle management.

Deliver:

- Radarr adapter;
- Sonarr adapter;
- Lidarr adapter;
- domain search merged with local catalog;
- request state machine;
- admin approval;
- per-user auto-approval capability;
- admin domain acquisition defaults;
- add/monitor/unmonitor/search/delete operations;
- availability/download state;
- manual release search and grab;
- manager-owned scoring/rejection display;
- exact manager binding by file record/path and stable IDs;
- manager ownership state model covering managed, confirmed unmanaged, and unresolved files, including sticky historical ownership when a manager cannot currently be reached;
- TV provider/order mapping from logical episodes to the exact Sonarr episode/file identities used for manager actions instead of assuming numeric season/episode equality;
- durable media-operation coordinator for destructive commands, with target file-generation capture, local conflict leases, and final ownership/identity/protection revalidation;
- no fallback from a failed manager-routed destructive action to direct filesystem deletion.

Exit condition:

- with manually configured test services, the UI can request, acquire, bind, unmonitor, delete, and reacquire Movies/Shows/Music without Docker management yet; manager outages/path-mapping failures block destructive work instead of reclassifying files as unmanaged; TV fixtures with provider numbering differences do not cause a season action to target the wrong manager episodes.

## Phase 12. Bazarr, Prowlarr, and NZBGet APIs

Deliver:

- Bazarr connection and subtitle acquisition actions;
- Prowlarr health, indexer problem visibility, test, enable/disable;
- NZBGet queue/progress/rate/history;
- NZBGet pause/resume/remove/global pause/speed limit controls;
- combined acquisition/download state in first-party UI;
- links to native service UIs for advanced configuration.

Exit condition:

- an admin can diagnose the common indexer/download problems and run common operational actions without leaving Thelxinoe.

## Phase 13. Docker controller and managed stack

Turn the API integrations into a managed self-hosted stack.

Deliver:

- constrained server/controller protocol;
- curated service templates;
- install supported service;
- adopt compatible existing container;
- exclusive Docker-ownership checks for adoption, requiring release of the old Compose service definition and rejecting other detected orchestrators;
- copying existing config into managed storage, retaining the exact image and integration identity, with original-service recovery after a failed transfer;
- labels/stable managed-service IDs;
- canonical `/media/...` mounts for created services;
- appdata under state root;
- Docker configuration ownership and drift detection;
- controller-owned persistent first-party deployment descriptor containing the accepted immutable server/controller digests, recreate specs, and controller generation outside server SQLite;
- persistent deployment Compose project with a controller-maintained `compose.override.yaml` that pins the accepted immutable first-party digests from the deployment descriptor, so normal `docker compose up` in the deployment directory cannot recreate stale bootstrap images;
- automatic service wiring;
- native admin-facing URL tracking;
- stable-only release discovery;
- immutable digest resolution.

Exit condition:

- a fresh Thelxinoe install can create an optional working Radarr/Sonarr/Lidarr/Bazarr/Prowlarr/NZBGet stack and wire it together without manual Docker work.

## Phase 14. Managed-service update safety

Deliver:

- Automatic, Notify, Manual policies;
- stable-only candidate release discovery;
- per-service consistent appdata snapshot strategy;
- disposable cloned-appdata preflight on an isolated network with no production service/Docker/host access and no arbitrary egress;
- controlled disposable/stub dependency fixtures for adapter contracts;
- adapter contract tests;
- candidate status compatible/incompatible/unable-to-verify;
- pre-update consistent appdata recovery snapshot;
- isolated validation of migrations/startup/API contract against real live-state appdata before production activation;
- explicit production activation boundary after which external side effects are possible;
- live replacement only after isolated validation succeeds;
- health/API validation;
- rollback of image/spec and appdata while still on the isolated side of the activation boundary;
- idle detection;
- maintenance window;
- blocked-update/admin notification flow.

Exit condition:

- a test service can upgrade, fail preflight, fail post-update, and roll back without corrupting live state.

## Phase 15. Retention engine

Build retention only after manager bindings and watched state are mature.

Deliver:

- Movie and TV retention-trigger user configuration;
- complete-season eligibility logic;
- exclusion of airing/uncertain seasons and default Season 0 exclusion;
- grace periods;
- admin Keep;
- pending-retention queue;
- cancel/delete-now;
- reuse of the Phase 11 durable media-operation coordinator and local conflict leases;
- final file-generation, manager-ownership, Keep/protection, activity, and target-set revalidation immediately before destructive execution;
- per-library-root opt-in for automatic direct deletion of confirmed-unmanaged files; without it, retention stops at an admin-executed pending action for those files;
- manager deletion and unmonitoring;
- manager import-list exclusion handling;
- reacquisition path that reverses retention state;
- unmanaged direct deletion;
- YouTube interest-based cleanup.

Exit condition:

- retention can safely delete watched test media, cannot delete incomplete/active/protected media, and can later reacquire retained managed media.

## Phase 16. Media segments and automatic skipping

Deliver:

- provider-independent segment schema;
- local recurring-audio analysis pipeline;
- persisted fingerprints/results;
- TheIntroDB-style external provider integration;
- confidence/source tracking;
- manual correction UI;
- per-user Auto/Ask/Ignore for Intro/Recap/Credits/Preview;
- first-party skip UI and automatic seeks;
- Jellyfin Media Segments compatibility;
- low-priority scheduling and reanalysis controls.

Exit condition:

- representative shows get usable intro/credit segments without blocking library work, and Wholphin receives those segments through its normal Jellyfin path.
