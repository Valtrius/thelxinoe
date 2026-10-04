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

2. Create `server`, `cache` and `deployment` under the state root, the backup directory, and `movies`, `tv`, `music` and `downloads` under the media root. Grant the account read/write access; UID/GID settings do not change existing ownership. Keep state/backups outside media. **For adoption, prepare the paths and network below before starting Thelxinoe.**
3. Run `docker compose up --build -d` from the repository. Open `http://<server-IP>:8484` and create the first administrator. See [deployment and recovery](docs/OPERATIONS.md) for HTTPS and storage details.

### Create services that do not exist yet

1. Open **Settings → Media services**, select a service, and use **Install and own it → Install …**. Choose an unused local port. Thelxinoe creates its container and configuration.
2. Install **NZBGet** for Usenet downloads, **Prowlarr** for indexers, and **Radarr** for movies or **Sonarr** for shows. **Lidarr**, **Bazarr**, **Seerr** and **Recyclarr** are optional.
3. Open services through their Thelxinoe links. Add your Usenet provider in NZBGet and indexers in Prowlarr. Review Radarr/Sonarr acquisition defaults and check **Download clients**, Prowlarr's **Applications**, and Bazarr's **Media managers** connections.
4. Use the library pages for playback. Install Seerr to search and request movies/shows from **Discover**. Configure **Settings → Backups** and keep a copy off the server.

## Adopt existing services

**Connect** integrates a service while its original owner controls the container. **Take ownership** gives Thelxinoe control of start/stop, updates and backups through a managed copy. Adopt one service at a time.

### 1. Check compatibility and back up

- Radarr, Sonarr, Lidarr, Bazarr, Prowlarr and NZBGet must run on the **same Docker engine** as Thelxinoe, using stable **LinuxServer Linux x86-64 images**. Jellyfin/Jellyseerr cannot be adopted.
- Keep the installed image during preparation; pin its digest if using `latest`. Adoption retains that image.
- Keep image-default commands/users, non-root numeric `PUID`/`PGID`, and only `PUID`, `PGID`, `TZ`, `UMASK` environment overrides. Remove custom hostnames, health checks, DNS, security profiles, devices, extra mounts and privileged settings. Cluster-managed containers are unsupported.
- Finish downloads/imports, stop services, and back up their configuration directories and Compose files before changing paths.

### 2. Use one shared media mount

Keep each service's writable `/config` bind mount outside media and Thelxinoe's deployment directory. Replace separate `/movies`, `/tv` and `/downloads` mounts with **one writable bind mount of the entire media root**:

```yaml
# In the existing service's Compose definition; Radarr example:
volumes:
  - /tank/appdata/radarr:/config:rw
  - /tank/media:/media:rw
```

Prowlarr needs only `/config`. Adoption does not support named volumes or extra mounts.

Recreate affected containers from their old Compose project using the same image, then update their native settings:

| Setting                                      | Required container path |
| -------------------------------------------- | ----------------------- |
| Radarr root folder and existing movie paths  | `/media/movies`         |
| Sonarr root folder and existing series paths | `/media/tv`             |
| Lidarr root folder and existing artist paths | `/media/music`          |
| Bazarr's movie/show paths                    | Match Radarr/Sonarr     |
| NZBGet `MainDir`                             | `/media/downloads`      |

Move existing media under the shared root while services are stopped: for example, `/tank/shows` becomes `/tank/media/tv`. Bulk-edit existing library paths, then remove unused old root folders. When files are already in place, choose **not to move files**. In NZBGet, replace `/downloads` prefixes with `/media/downloads` in destination/intermediate/category paths; preserve subfolder names and `${MainDir}` references. Remove obsolete remote path mappings and verify the library/download locations.

### 3. Share one Docker network

Each adopted container must use **only Thelxinoe's server network**. To reuse an existing network, create `compose.override.yaml` beside Thelxinoe's `compose.yaml`:

```yaml
networks:
  default:
    external: true
    name: media_network
```

Use the actual name from `docker network ls`, including any Compose project prefix. Choose the network and media root before first initialization; later `.env`/mount edits do not update the saved deployment layout.

### 4. Connect, review and transfer

1. Keep the original running. In **Settings → Media services → Connect an existing container**, select it and enter its standard internal HTTP port, API key and existing URL Base. For NZBGet, enter its current username/password; an empty password is accepted.
2. Click **Review ownership transfer** and check the configuration paths and image. For NZBGet, select log rotation and outgoing TLS verification when offered. If TLS verification is unavailable, set `CertStore` to a readable certificate bundle, save/reload, and review again. These checks protect Usenet/HTTPS connections even through Thelxinoe.
3. Disable the service in the **old Compose file** and any scripts/updaters that could recreate it. Leave the original running for Thelxinoe to stop. Confirm **The previous Compose definition is disabled**, then click **Take ownership**.
4. Wait for completion. Thelxinoe disables the original's restart policy, stops it, copies its appdata and starts the replacement. Original configuration is retained; media stays in the shared directory.
5. Open the replacement through Thelxinoe and test its library and service connections. Radarr/Sonarr/Lidarr/Prowlarr/Bazarr switch to Thelxinoe authentication and `/services/<kind>` URL Bases, with published ports removed. NZBGet retains published ports but receives login `thelxinoe` and a random password. Thelxinoe updates its managed connections; update other clients yourself and explicitly connect remaining service links.

Never run the original alongside its replacement. Avoid `docker compose down`/`--remove-orphans` on the old project while retaining the original for recovery. For interrupted transfers, resolve the error and use **Retry setup** or **Restore original** when offered. Completed transfers use managed backups/recovery.

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
