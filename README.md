# Thelxinoe

[![CI](https://github.com/Valtrius/thelxinoe/actions/workflows/ci.yml/badge.svg?branch=develop)](https://github.com/Valtrius/thelxinoe/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![GitHub stars](https://img.shields.io/github/stars/Valtrius/thelxinoe)](https://github.com/Valtrius/thelxinoe/stargazers)
[![GitHub issues](https://img.shields.io/github/issues/Valtrius/thelxinoe)](https://github.com/Valtrius/thelxinoe/issues)
[![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Svelte](https://img.shields.io/badge/Svelte-FF3E00?logo=svelte&logoColor=white)](https://svelte.dev/)

Self-hosted movies, shows, music, YouTube, Twitch and Kick. Rust server, Svelte web/PWA and Windows Tauri app. [MIT](LICENSE).

## Start using Thelxinoe

### Set up the server

1. On your media host, use Docker with Linux x86-64 containers. Copy [`.env.example`](.env.example) to `.env`. Replace `<PUID>`/`<PGID>` with your media account's numeric IDs (`id -u <user>` / `id -g <user>`) and adjust the example paths:

   ```dotenv
   THELXINOE_UID=<PUID>
   THELXINOE_GID=<PGID>
   THELXINOE_STATE_ROOT=/tank/appdata/thelxinoe
   THELXINOE_MEDIA_ROOT=/tank/media
   THELXINOE_BACKUP_ROOT=/tank/backups/thelxinoe
   THELXINOE_LISTEN=0.0.0.0
   ```

2. Create `server`, `cache` and `deployment` under the state root, the backup directory, and `movies`, `tv`, `music` and `downloads` under the media root. Grant the account read/write access; UID/GID settings do not change existing ownership. Keep state/backups outside media. For existing services, keep their paths and prepare shared network access below.
3. Run `docker compose up --build -d` from the repository. Open `http://<server-IP>:8484` and create the first administrator. See [deployment and recovery](docs/OPERATIONS.md) for HTTPS and storage details.
4. Later, after rebuilding or changing `.env` or `compose.override.yaml`, run `docker compose up --build -d`; the controller waits for the new containers to be accepted. Run `docker compose run --rm controller accept-local`, review the listed changes, type `ACCEPT`, then run `docker compose up -d` again. Pointing Thelxinoe at a different state folder starts a different deployment.

### Create services that do not exist yet

1. Open **Settings → Media services**, select a service, and use **Install and own it → Install …**. Choose an unused local port. Thelxinoe creates its container and configuration.
2. Install **NZBGet** for Usenet downloads, **Prowlarr** for indexers, and **Radarr** for movies or **Sonarr** for shows. **Lidarr**, **Bazarr**, **Seerr** and **Recyclarr** are optional.
3. Open services through their Thelxinoe links. Add your Usenet provider in NZBGet and indexers in Prowlarr. Review Radarr/Sonarr acquisition defaults and check **Download clients**, Prowlarr's **Applications**, and Bazarr's **Media managers** connections.
4. Use the library pages for playback. Install Seerr to search and request movies/shows from **Discover**. Configure **Settings → Backups** and keep a copy off the server.

## Bring existing services to Thelxinoe

Each service is in one of two stages:

- **Connected**: Thelxinoe only validates an externally managed deployment: its container identity, health, version and API key. It never writes to the service, opens it, links it or manages its container. Requests, downloads, library sync, links and backups need an Owned service.
- **Owned**: Thelxinoe manages the deployment, either because you installed it here or because you took ownership of a Connected one. Ownership never adopts the original container in place: Thelxinoe stops the original, copies its configuration and runs a managed replacement.

### 1. Check compatibility

- Use the same Docker engine as Thelxinoe and a stable Linux x86-64 release no older than the tested minimum: Radarr 6.4.4.10685, Sonarr 4.0.20.3014, Lidarr 3.1.0.4875, Bazarr 1.6.2, Prowlarr 2.6.5.5623 and NZBGet 26.3 (LinuxServer images), Seerr 3.5.0 (`ghcr.io/seerr-team/seerr` or `seerr/seerr`) and Recyclarr 8.7.2. A newer release of the same major version is kept; an older supported build is upgraded to the tested image, which the review shows. Preview, development and other major versions are refused.
- `/config` (Seerr's configuration folder, Recyclarr's `/config`) must be its own host folder or Docker volume, used by no other container and not overlapping media or Thelxinoe's storage. Media and download folders are kept in place and never copied. Other writable folders, privileged or host-namespace containers, containers sharing another container's network (such as a VPN), Docker socket mounts, auto-remove (`--rm`) containers and unsupported environment or startup options are refused with the reason.
- The original must be running, idle and reachable with its API key.
- The replacement runs as Thelxinoe's `THELXINOE_UID`/`THELXINOE_GID`. Only the copied configuration is given to that identity. If the original used another PUID/PGID, the review names any media or download folder Thelxinoe's identity can't read or write. Adjust that folder's permissions yourself (for example, give Thelxinoe's group access), then review again. Thelxinoe never changes media ownership.

### 2. Connect it

1. Share a Docker network with the existing service. To reuse one, add `compose.override.yaml` beside Thelxinoe's `compose.yaml`:

   ```yaml
   networks:
     default:
       external: true
       name: media_network
   ```

   Use its actual name from `docker network ls`. Choose Thelxinoe's storage/network layout before first initialization.

2. For playback, mount the same media into Thelxinoe. Service paths can differ. For example, Radarr's `/tank/media/movies:/movies` and Thelxinoe's `/tank/media:/media` identify the same movies. You can add separate folders such as `/tank/shows:/media/tv` beneath `/media`. Creating fresh media services requires the single shared `/media` mount without child mounts.
3. Open **Settings → Media services**, select the service and use **Connect existing …** with its container, internal port, URL Base and API key (for NZBGet, its username and password). For Recyclarr, choose its container under **Connect an existing job container**; Thelxinoe validates its configuration read-only. **Test connection** repeats the read-only check; **Forget connection** removes only Thelxinoe's record.

A Connected container may be recreated by Compose or an updater. Thelxinoe follows the replacement when it keeps the same container name, mounts, API identity and credentials. Otherwise choose its new container in **Edit connection**.

### 3. Take ownership

**Take ownership** opens a guided transfer. You can close it at any time; the work continues and the service shows **Open ownership transfer**.

1. **Review changes**: the original and replacement images, storage, identity, published ports (rebound to this host's loopback address), aliases and every application setting the copy changes, such as URL Base and authentication. Links with Owned services are created after completion; an existing matching record is offered for **Import existing** instead of being overwritten. Dependencies Thelxinoe doesn't own stay external.
2. **Confirm automation is retired**: Thelxinoe shows the updaters and containers with Docker access it can see. Declare that cron jobs, systemd timers, CI/CD, configuration management and manual Compose commands won't touch the original. For a Compose project, choose **Detach only this service** (remove it from the project's files and never run `down`, `rm` or `--remove-orphans`) or **Retire the whole project's automation**.
3. **Confirm the transfer**. Thelxinoe disables the original's restart policy and stops it, copies its configuration to a verified snapshot and validates a disposable copy with synthetic media and no network route. It then starts the managed replacement, commits ownership under an operational hold, runs only the setup you reviewed and completes when every required check passes. Normal operations resume only then.

The original container and its configuration folder are kept, stopped with restart disabled. After completion, **Remove the stopped original container** deletes only that container, never its configuration.

If a step fails before the replacement starts, Thelxinoe restores the original automatically. After the replacement has started, normal operations stay held and **Open transfer recovery** shows what changed since activation, what recovery can't undo and the available choices: retry setup and checks, return to the original, restore the verified snapshot, or suspend for manual resolution. A replacement never starts while the original can run. Returning to the original removes the stopped replacement container and keeps its configuration as a preserved generation, so you can take ownership again later. Restoring the snapshot builds a new replacement and runs the reviewed setup again for it. A recovery you approve finishes even if the page closes or Thelxinoe restarts. Service updates, backups and lifecycle actions wait while a service is held.

Read-only supplementary mounts, such as a certificate folder or a single custom script, are copied into managed storage and mounted read-only at the same paths.

## Development

Development requires Rust 1.96+, Node 24+ and pnpm 10.33.2, Perl, FFmpeg/FFprobe 7.1+ and Docker with Linux containers.

```sh
pnpm install --frozen-lockfile
pnpm run dev
```

Open http://127.0.0.1:5173 and create the first administrator. The server runs on port 8484; development state stays in `.local`. Ctrl+C stops both processes.

| Command                | Purpose                                                                                                                        |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `pnpm run dev:web`     | Frontend only                                                                                                                  |
| `pnpm run dev:mock`    | UI preview with an admin account and disposable mock data on a free port; no server or Docker required                         |
| `pnpm run dev:desktop` | Windows app; requires a running server                                                                                         |
| `pnpm run dev:fresh`   | Windows: new empty profile at http://127.0.0.1:18486; previous runs remain in `.local/dev-runs`                                |
| `pnpm run dev:online`  | Windows: reuse the saved online profile at https://localhost:22443, with LAN access and UDP discovery; containers keep running |
| `pnpm run validate`    | Formatting, lint, Rust/web checks, product tests and web build                                                                 |
| `pnpm run ci`          | Parallel local [CI phases](scripts/ci.mjs), including Docker fixtures and Windows desktop checks                               |
| `pnpm run ci:local`    | Windows CI with a completion notification, live summary and lane logs under `.local/ci`                                        |
| `pnpm run ci:status`   | Latest local CI lane results and report paths                                                                                  |

New `dev:online` profiles use separate controller storage, even when a data folder is reused after a reset. Keep `server/controller-storage-id` with the saved database and key so later launches reconnect to the same controller state.

For mock UI reviews, run `pnpm run dev:mock` after installing dependencies and open the URL printed at startup. Set `THELXINOE_MOCK_PORT` to choose a fixed port. Restart after source edits. FFmpeg enables the local sample player. Changes last until restart; `POST /__mock/reset` on the preview restores the sample data. Movie and TV details are available at `/movie/11` and `/tv/22`. Run `pnpm run test:ui:mock` for the browser checks and screenshots in `playwright-report/mock` and `test-results/mock`.

`dev:online` also listens at `http://<LAN-IP>:18888` and `https://<LAN-IP>:22443`. It selects an active LAN interface; set `THELXINOE_ONLINE_LAN_IP` or run `pnpm run dev:online -LanAddress 192.168.1.10` to choose another address assigned to this PC. Other PCs and compatible clients can discover the server on UDP 7359. Allow TCP 18888/22443 and UDP 7359 from the local subnet through Windows Firewall. Link provider accounts from https://localhost:22443 on the server PC.

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
