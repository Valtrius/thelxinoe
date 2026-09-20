# Operations and release acceptance

These are the accepted v1 delivery criteria. Current results and supported limits are in [implementation status](../STATUS.md) and the [release checklist](../RELEASE_CHECKLIST.md).

## Phase 17. Admin operations, notifications, and backups

Deliver:

- admin health dashboard;
- storage/cache usage;
- service/integration health;
- Prowlarr/indexer alerts;
- active playback/transcode/download views;
- in-app notification center;
- browser/Tauri realtime notifications;
- manual backup and restore using the same component-specific consistency primitives as update snapshots;
- encrypted portable backup archives;
- pre-update snapshot UI/state;
- restore of the first-party deployment descriptor alongside state so the compatible server/controller generation can be recreated offline;
- redacted diagnostic bundle;
- recent error view;
- user/device administration;
- user deletion cleanup.

Exit condition:

- an admin can diagnose, back up, restore, and manage common failure cases without shell access.

## Phase 18. First-party update system

Deliver:

- signed release-manifest format;
- immutable server/controller digests;
- desktop artifact hashes;
- migration metadata plus recovery mode metadata that distinguishes in-place rollback from full pre-update state restore;
- server Automatic/Notify/Manual policy;
- idle and maintenance-window handling;
- verified server pre-update rollback bundle using SQLite online backup or quiesced state capture rather than raw live-WAL copying;
- controller-owned offline restore path that can recover the old deployment descriptor/images/state without starting the new server or parsing its migrated database;
- Automatic-install gating that requires the current controller to have a tested unattended recovery path, including for forward-only database migrations;
- pre-activation server validation mode with scheduled/external side-effect work disabled and no live integration/Docker mutation access;
- fenced controller self-handoff using the persistent generation/lease so only one controller can mutate Docker;
- Tauri updater tied to the same product release;
- client/server compatibility check and clear update-required state.

Exit condition:

- a full test release can update server/controller/web and Tauri from one version line, and controlled failure cases leave a recoverable installation.

## Phase 19. PWA and remote-access hardening

Deliver:

- PWA manifest and icons;
- standalone install behavior;
- final public-base-URL audit across auth/invitations/playback links;
- full trusted-proxy and secure-cookie regression suite building on the earlier phase-level proxy tests;
- CORS allowlist support only where required;
- full reverse-proxy WebSocket/HLS regression suite building on the earlier transport/playback tests;
- remote bitrate defaults;
- responsive mobile usability pass.

Exit condition:

- the web app works through a normal HTTPS reverse proxy on desktop and responsive Chromium mobile without special client-side routing hacks.

## Phase 20. V1 compatibility and release hardening

Finish the product by validating the complete system rather than adding new features.

Deliver:

- end-to-end clean-install test from Compose;
- first-run wizard test;
- upgrade-from-previous-release fixture;
- backup/restore fixture;
- offline restore fixture after an intentionally failed forward-only server migration;
- stale-bootstrap recreation fixture proving the persisted deployment descriptor prevents accidental first-party downgrade;
- Wholphin compatibility suite;
- another Jellyfin media-client compatibility suite;
- Jellyfin compatibility credential redaction and URL/auth-transport fixture;
- representative browser playback matrix;
- browser and MPV gapless/ReplayGain music fixtures;
- Windows Tauri playback/tool/update matrix;
- quota/rate-limit tests;
- retention destructive-action tests;
- manager-outage, stale-binding, provider-order mismatch, and file-replacement-before-delete fixtures;
- service update rollback tests;
- managed-service preflight isolation fixture proving a candidate cannot reach live peer services;
- auth/proxy/security review;
- diagnostic redaction tests;
- deployment/reverse-proxy/provider/storage/backup documentation;
- release checklist.

Exit condition:

- the supported Linux x86-64 server, Windows x64 Tauri client, web/PWA clients, and Jellyfin media clients pass the documented v1 workflows end to end.
