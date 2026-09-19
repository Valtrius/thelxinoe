# Implementation status

Updated 2026-09-19. This is a runnable development milestone, not a completed v1. The product specifications remain the full target.

| Phase                | State                        | Evidence / remaining work                                                                                                                                                                                                                                         |
| -------------------- | ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0 — Foundation       | Verified locally             | Cargo/Svelte/Tauri workspaces, shared version checks, Linux images, Compose startup, browser bundle, Windows installer and native runtime. Controller uses only its private Unix socket. CI definitions are written; hosted CI has not run.                       |
| 1 — Identity         | Verified locally             | Setup, accounts, Argon2id, encrypted secrets, master key, sessions, capabilities, proxy trust, timezone, browser login through Caddy HTTPS, native keyring persistence, restart and revocation.                                                                   |
| 2 — API and realtime | Verified locally             | Shared backend client, normalized errors, scoped grants, event replay/reconnect, persistent bounded job worker. Browser proxy tests, native reconnect after a server restart, and interrupted-job idempotency test pass.                                          |
| 3 — Local catalog    | Implemented and tested       | Typed roots, FFprobe/tags, opaque identities, editions, specials, multi-episode files, watchers/reconciliation, progress events and browsing. Replacement, rename, duplicate-copy, removal and failed-scan behavior covered by tests.                             |
| 4 — Metadata         | Verified with live providers | TMDB movie/show matching, collections, explicit episode mapping, MusicBrainz artist/album matching, TMDB/CAA artwork and preserved manual corrections pass against real providers through HTTPS.                                                                  |
| 5 — Local playback   | Implemented and tested       | Direct HTTP ranges; HLS remux/conversion and seeks decoded in Chromium through HTTPS; audio/subtitle preferences and selection; scoped grants; independent users, edition resume, replacement/restart recovery; sample-continuous FLAC sequencing and ReplayGain. |
| 6–20                 | Not implemented              | MPV, additional user media state, Jellyfin compatibility, online providers, acquisition services, managed Docker lifecycle, retention, segments, backups, signed updates, PWA and release hardening remain open.                                                  |

## Provider configuration

TMDB uses an application credential. The development implementation accepts an administrator-configured token in encrypted server storage. Regular library users never need a TMDB account or key. A project-provided credential could make future releases work without per-installation setup; no such credential has been supplied or bundled.

MusicBrainz public metadata does not require an API key. The supplied maintainer contact is configured in the application User-Agent.

The private inputs `.local/tmdb-token` and `.local/musicbrainz-contact` were supplied and imported into the main Docker deployment. The TMDB token is encrypted in server storage. The separate `.local/tmdb-api` key is unnecessary when using the Read Access Token. Stop the destination server before using offline import helpers.

Google, Twitch and Kick application settings were found in YouTwitch's Windows Credential Manager entry and imported into encrypted Thelxinoe development storage. Viewer OAuth tokens were not imported. Live account linking and playback belong to phases 9 and 10 and have not been validated.

## Runtime and evidence

- Main Compose deployment: `http://127.0.0.1:8484`, data in `.local/docker`, awaiting creation of the administrator account. Setup code: `.local/docker/server/secrets/setup-token`.
- Isolated HTTPS test deployment: `compose.test.yaml`, Caddy at `https://localhost:9443`, direct test server at `127.0.0.1:18484`. Uses generated media and test-only accounts in separate named volumes.
- Windows installer: `target/release/bundle/nsis/Thelxinoe_0.1.0_x64-setup.exe`.
- Screenshots: `.local/library.png`, `.local/desktop-library.png`, `.local/proxy-settings.png`.
- Live metadata evidence: `.local/live-metadata-result.json`, `.local/live-metadata.png`; isolated `thelxinoe-live` Compose deployment on HTTPS port 19443, using test accounts and generated files.
- Playback evidence: `.local/playback-result.json`, `.local/playback-direct.png`; isolated `thelxinoe-playback` deployment on HTTPS port 20443. Native MPV playback is the next phase.
- Android TV API 34 AVD `Thelxinoe_TV` created under `.local/android-avd`; Wholphin 1.0.8 installed and launched to its Add Server screen. **No Wholphin compatibility claim:** the Jellyfin adapter and playback are not implemented yet.

See [testing instructions](TESTING.md) for commands. Secrets, media, emulator images and test accounts are excluded from Git.
