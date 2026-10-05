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

### Create services that do not exist yet

1. Open **Settings → Media services**, select a service, and use **Install and own it → Install …**. Choose an unused local port. Thelxinoe creates its container and configuration.
2. Install **NZBGet** for Usenet downloads, **Prowlarr** for indexers, and **Radarr** for movies or **Sonarr** for shows. **Lidarr**, **Bazarr**, **Seerr** and **Recyclarr** are optional.
3. Open services through their Thelxinoe links. Add your Usenet provider in NZBGet and indexers in Prowlarr. Review Radarr/Sonarr acquisition defaults and check **Download clients**, Prowlarr's **Applications**, and Bazarr's **Media managers** connections.
4. Use the library pages for playback. Install Seerr to search and request movies/shows from **Discover**. Configure **Settings → Backups** and keep a copy off the server.

## Adopt existing services

**Connect** gives Thelxinoe access to an application's API. **Take ownership** adds container lifecycle management. Adoption keeps the existing container, image, mounts, ports, credentials, authentication, URL Base and application settings. It does not restart the service or move files.

### 1. Check compatibility

- Use the same Docker engine as Thelxinoe and a supported stable LinuxServer Linux x86-64 image: Radarr, Sonarr, Lidarr, Bazarr, Prowlarr or NZBGet. Cluster-managed, privileged and host-system deployments are outside the supported scope. Jellyfin/Jellyseerr cannot be adopted.
- Back up your configuration and deployment definition. Keep your existing `PUID`/`PGID`, storage, network connections and published ports.
- The review reports available operations. Lifecycle management can work even when a storage layout cannot be backed up. Imported deployments currently use update checks only; recreation and automatic updates remain disabled until faithful recovery is supported.

### 2. Give Thelxinoe access

1. Share a Docker network with the existing service. To reuse one, add `compose.override.yaml` beside Thelxinoe's `compose.yaml`:

   ```yaml
   networks:
     default:
       external: true
       name: media_network
   ```

   Use its actual name from `docker network ls`. Preserve the service's other networks. Choose Thelxinoe's storage/network layout before first initialization.

2. For playback, mount the same media into Thelxinoe. Service paths can differ. For example, Radarr's `/tank/media/movies:/movies` and Thelxinoe's `/tank/media:/media` identify the same movies. Keep Thelxinoe's `/media` bind mount; you can add separate folders such as `/tank/shows:/media/tv` or shared named volumes beneath it. Thelxinoe must have permission to read the files. Creating fresh media services requires the single shared `/media` mount without child mounts.
3. Keep native root folders and download paths. NZBGet can keep `MainDir=/downloads` and `${MainDir}/complete`; no conversion to `/media` is required.

### 3. Transfer ownership

1. Open **Settings -> Media services -> Connect an existing container**. Select its full container identity, internal HTTP port, existing URL Base and API key. For NZBGet, enter the current username/password. A stopped container may be connected, with API access unverified until you start it.
2. Click **Review ownership transfer**. Check the retained image/configuration location, available operations and any warnings. Security hardening is a separate change in native settings.
3. Retire external deployment automation and updaters. **If the container belongs to Compose, retire the entire old project.** Removing just its YAML service is insufficient: subsequent `down` or `--remove-orphans` operations can remove the adopted container or shared resources. If that project must continue running, keep API integration and defer ownership until a separately reviewed detachment is supported.
4. Confirm the project is retired and click **Take ownership**. Wait for completion. The container's identity and running/stopped state stay the same; the existing integration remains connected.
5. Test API access and playback. Use **Edit connection → Test connection → Save connection** to correct credentials, the internal port or URL Base. Under acquisition defaults, explicitly choose an existing root folder and quality profile; Lidarr also needs a metadata profile. Review each application connection, including Seerr's **Media managers**, before enabling Thelxinoe to manage it. Existing manual and disabled connections remain untouched. Auto-delete requires a separate opt-in. If proxy access is unavailable, set the service's **Native web address**; it keeps its own login.

Use **Release ownership** to stop Thelxinoe's container management while retaining the container, configuration, media and API integration. If release is interrupted, use **Retry release**. External Docker changes pause affected management operations until reviewed. For an interrupted registration, use **Retry setup**; registration retries keep the same operation and container.

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
| `pnpm run dev:desktop` | Windows app; requires a running server                                                                                         |
| `pnpm run dev:fresh`   | Windows: new empty profile at http://127.0.0.1:18486; previous runs remain in `.local/dev-runs`                                |
| `pnpm run dev:online`  | Windows: reuse the saved online profile at https://localhost:22443, with LAN access and UDP discovery; containers keep running |
| `pnpm run validate`    | Formatting, lint, Rust/web checks, product tests and web build                                                                 |
| `pnpm run ci`          | Parallel local [CI phases](scripts/ci.mjs), including Docker fixtures and Windows desktop checks                               |
| `pnpm run ci:local`    | Windows CI with a completion notification, live summary and lane logs under `.local/ci`                                        |
| `pnpm run ci:status`   | Latest local CI lane results and report paths                                                                                  |

New `dev:online` profiles use separate controller storage, even when a data folder is reused after a reset. Keep `server/controller-storage-id` with the saved database and key so later launches reconnect to the same controller state.

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
