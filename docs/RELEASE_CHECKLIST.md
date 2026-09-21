# V1 release checklist

The supported baseline is Linux x86-64 Docker, Windows x64 Tauri, current Chromium web/PWA and the specific Jellyfin clients listed in [JELLYFIN.md](JELLYFIN.md). Hardware transcoding, other desktop platforms, ARM64, offline media and broader Jellyfin server compatibility remain outside v1.

## Required checks

- Run `npm run validate`, `cargo fmt --all --check`, Windows workspace Clippy/tests and Linux controller/server tests. Linux-specific ownership, archive and process tests cannot be replaced by Windows-only Cargo results.
- Build both Linux images and the production Windows NSIS installer from the same shared product version. Keep test publisher overrides and test application identities out of production builds.
- Start an empty Compose deployment; create the administrator through the wizard, verify secure login and device revocation, and restart it without losing state.
- Run a signed previous-release upgrade, failed forward migration, offline controller recovery, encrypted earlier-generation restore and actual stale-bootstrap Compose recreation. Keep old recovery images until those checks pass.
- Test the signed Windows updater against incorrect metadata, a corrupt artifact and a successful upgrade with the device credential retained.
- Run the browser direct/remux/transcode, seek, subtitle, music/gapless/ReplayGain and independent-user playback fixtures. Repeat HTTP Range, HLS and event reconnect through HTTPS.
- Run the documented Wholphin and Jellyfin Android TV matrix with their pinned versions. Reassess changed upstream protocol requirements before supporting a newer client. Record emulator limitations separately from server protocol failures.
- Run OAuth/quota/rate-limit fixtures and representative real YouTube/Twitch/Kick playback with the configured test accounts. Never pass viewer entitlement credentials into media extractors.
- Run acquisition, service health/actions, retention and manager deletion fixtures, including Keep/activity, stale ownership, file replacement and differing TV numbering. A manager outage must block destructive work.
- Run managed-service update/preflight isolation and recovery fixtures. Candidate networks must not reach live peers. State restore after production activation requires explicit action.
- Run user deletion, notifications, diagnostics/credential redaction, encrypted backup corruption and interrupted backup/restore tests.
- Check mobile layout, PWA installability/offline reconnect, same-origin security, explicit CORS, trusted forwarding and remote quality defaults. Review the installed application's update-required screen.
- Verify deployment, provider, storage, operations, recovery and remote-access documentation. Preserve the signed manifest, image identities, checksums, schema/recovery metadata and test results with release artifacts.

## Local evidence map

Private evidence lives in `.local` and is excluded from source control. The scripts and protocol tests are tracked so results can be reproduced against isolated fixtures.

| Workflow                            | Evidence / reproducible entry point                                                                                                                        |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Identity, proxy, event reconnect    | `tests/setup.spec.ts`, `tests/proxy.spec.ts`, server auth integration tests                                                                                |
| Indexing, metadata and artwork      | `manager-files-result.json`, `manager-tv-music-result.json`, catalog indexing and manager metadata tests                                                   |
| Browser playback and user state     | `scripts/test-playback.mjs`, `test-playback-tracks.mjs`, `test-user-media.mjs`, `playback-result.json`                                                     |
| Windows MPV and segments            | `native-playback-result.json`, `native-segments-result.json`, tracked native test scripts                                                                  |
| Jellyfin clients and credentials    | `scripts/test-jellyfin.mjs`, `jellyfin-result.json`, TV screenshots/history and [client matrix](JELLYFIN.md)                                               |
| Real online providers               | `youtube-live-result.json`, `youtube-refresh-result.json`, `youtube-native-stream-result.json`, `twitch-playback-result.json`, `kick-playback-result.json` |
| Acquisition and service integration | `acquisition-downloads-result.json`, `manager-files-result.json`, `manager-tv-music-result.json`, `support-actions-result.json`                            |
| Managed lifecycle and updates       | `managed-install-result.json`, `ownership-test-*/result.json`, `service-update-result.json`, `service-rollback-result.json`                                |
| Destructive retention and segments  | `retention-result.json`, `segments-result.json`, Rust ownership/retention/segment tests                                                                    |
| Administration and backups          | `operations-result.json`, `backup-interruption-result.json`, `admin-ui-result.json`                                                                        |
| Product upgrade and recovery        | `scripts/test-product-release.mjs`, `test-release-interruption.mjs`, `test-release-recreation.mjs`, `product-release-result.json`                          |
| PWA and remote delivery             | `scripts/test-pwa-remote.mjs`, `pwa-remote-result.json`, `pwa-mobile-settings.png`                                                                         |
| Native updater                      | `scripts/test-native-update.mjs`, `test-native-compatibility.mjs`, `native-update-result.json`, `native-compatibility-result.json`                         |
| Appearance and media motion         | `tests/appearance.spec.ts`, `ui-validation/screens-result.json`, `ui-validation/native-result.json`, `ui-validation/live-result.json`                      |

The selected Android emulator does not provide normal LAN broadcast discovery. Host UDP discovery and manual TV connection are tested; a physical LAN broadcast smoke check remains a deployment acceptance check. Wholphin's long, already-active subtitle cue behavior after seeking is documented in its compatibility matrix. Hosted CI and public artifact publication require a configured repository/runner/release host; local builds do not imply those services have run.
