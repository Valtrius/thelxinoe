# Thelxinoe

Self-hosted movies, shows, music, YouTube, Twitch and Kick. Rust server, Svelte web/PWA and Windows Tauri app. [MIT](LICENSE).

## Run

Development requires Rust 1.96+, Node 24+, FFmpeg/FFprobe and Docker with Linux containers.

```sh
npm ci
npm run dev
```

Open http://127.0.0.1:5173 and create the first administrator (password: 12+ characters). The server runs on port 8484; development state stays in `.local`. Ctrl+C stops both processes.

| Command                        | Purpose                                                                                                                      |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `npm run dev:web`              | Frontend only                                                                                                                |
| `npm run dev:desktop`          | Windows app; requires a running server                                                                                       |
| `npm run dev:fresh`            | Windows: new empty profile at http://127.0.0.1:18486; previous runs remain in `.local/dev-runs`                              |
| `npm run dev:online`           | Windows: reuse existing `.local/online/server` database and key at https://localhost:22443; containers keep running          |
| `docker compose up --build -d` | Deploy using [Compose](compose.yaml) and [.env.example](.env.example); read [storage and recovery](docs/OPERATIONS.md) first |
| `npm run validate`             | Formatting, lint, Rust/web checks, unit tests and web build                                                                  |
| `npm run ci`                   | [CI phases](scripts/ci.mjs), including Docker fixtures and Windows desktop checks                                            |

Open each provider page for connection instructions and application setup. Administrators configure credentials once; each user connects YouTube/Twitch or tracks Kick channels. Local metadata comes from connected Radarr/Sonarr/Lidarr services; files remain usable without them.

## Source map

Code and tests define behavior. Keep docs to commands, operational prerequisites and limits; put setup help beside the controls it explains.

| Area                         | Entry points                                                                                                                                             |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Server/API/configuration     | [router](apps/server/src/lib.rs), [configuration](apps/server/src/config.rs), [.env.example](.env.example)                                               |
| Database                     | [schema](crates/database/schema.sql), [runtime](crates/database/src/lib.rs), [server SQL](apps/server/src/storage)                                       |
| Identity and permissions     | [authentication](crates/auth/src), [security](apps/server/src/security.rs), [capabilities](crates/core/src)                                              |
| Catalog and playback         | [catalog](crates/catalog/src), [playback](crates/playback/src), [server playback](apps/server/src/playback.rs)                                           |
| Online providers             | [server](apps/server/src/online), [pages](frontend/src/lib/providers), [setup instructions](frontend/src/lib/providers/ProviderSetupInstructions.svelte) |
| Jellyfin clients             | [adapter](apps/server/src/jellyfin), [protocol checks](scripts/test-jellyfin.mjs)                                                                        |
| Media services and retention | [manager integrations](apps/server/src/managers), [Docker controller](apps/docker-controller/src), [retention](apps/server/src/managers/retention.rs)    |
| Web and Windows UI           | [app](frontend/src/App.svelte), [shared controls](frontend/src/lib/ui), [desktop](apps/desktop/src)                                                      |
| Builds and releases          | [commands](package.json), [CI](.github/workflows/ci.yml), [Dockerfile](Dockerfile), [release protocol](crates/releases/src/lib.rs)                       |

## Limits

Supported targets: Linux x86-64 server, Windows x64 desktop and Chromium web/PWA. Transcoding uses software. Provider playback is public media only; account linking does not grant restricted playback. Offline media, casting, background Web Push, remote API-only media managers and full Jellyfin server compatibility are outside scope.

Wholphin 1.0.8 and Jellyfin Android TV 0.19.10 were locally checked for local media; online libraries were checked in Wholphin only. Wholphin can lose a long active subtitle cue after seeking. Universal audio conversion without playback-info is unsupported. LAN broadcast discovery still needs testing on a real LAN. Repeat client checks before claiming support for newer versions.

This is an unreleased schema baseline with no database upgrade runner. Release publication is not configured. See [deployment, backups and release commands](docs/OPERATIONS.md) and [test commands](docs/TESTING.md); current CI results are in [GitHub Actions](https://github.com/Valtrius/thelxinoe/actions/workflows/ci.yml).
