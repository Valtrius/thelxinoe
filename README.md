# Thelxinoe

A self-hosted media server with a shared Svelte web and Windows Tauri application.

See the [documentation index](docs/README.md) for product scope, architecture and operations.

The planned v1 workflows are implemented and locally validated. See [implementation status](docs/STATUS.md) for evidence and supported limits, and the [release checklist](docs/RELEASE_CHECKLIST.md) before distributing a release. Public artifact hosting and hosted CI are not configured in this checkout.

## Development

Requires Rust 1.96+, Node 24+, FFmpeg/FFprobe, and Docker with Linux containers.

```powershell
npm ci
npm run build
cargo run -p thelxinoe-server
```

Open http://127.0.0.1:8484 and choose a username and password to create the first administrator. Development data remains in `.local`. For frontend hot reload, run `npm run dev` in another terminal. Vite proxies the API and WebSocket to port 8484.

For a new empty Windows development instance on every launch:

```powershell
npm run dev:fresh
# Optional: another port and an already-built frontend
npm run dev:fresh -- -Port 18487 -SkipBuild
```

The launcher builds the frontend and runs the server at http://127.0.0.1:18486. Open that address and choose the first administrator's username and password. Each invocation creates separate state, cache and media folders under `.local/dev-runs/`, with no provider credentials or linked accounts. Ctrl+C stops the server. Earlier runs stay on disk, and existing deployments are preserved. Inherited `THELXINOE_*` settings are temporarily replaced and restored when the script exits. This serves the built web UI without hot reload or the Docker controller.

The underlying `scripts/dev-fresh.ps1` can also be invoked by its full path from another working directory. Run `npm ci` once before first use.

To reuse the existing online development profile with its saved provider credentials and account connections:

```powershell
npm run dev:online
# Optional: start the existing image without rebuilding
npm run dev:online -- -SkipBuild
```

This Windows launcher builds the server image (including the web UI), starts the `thelxinoe-online` Docker Compose project, and waits for it to become healthy at https://localhost:22443. Sign in with your existing Thelxinoe account. State persists in `.local/online/server`, including its database and encryption key; both must already exist. The containers keep running after the command exits. This serves the built UI without hot reload.

```sh
docker compose up --build -d
```

For Linux bind mounts, create the server, cache, and backup directories before startup and give them to uid/gid `10001:10001`. Configure persistent paths using `.env.example`. The Docker daemon is mounted only into the controller, which has no TCP network. The server connects over a private Unix socket.

The first account is an administrator. First-run setup is available while the server has no users; after that, additional accounts require an administrator. Passwords require at least 12 characters. Provider secrets use AES-256-GCM with a persistent master key outside SQLite. Losing that key loses the ability to decrypt provider credentials.

## Checks

```sh
npm run validate
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
npm run desktop:build
```

The canonical product version is `[workspace.package].version` in `Cargo.toml`. `npm run version:sync` updates npm, Tauri and Compose versions; builds reject drift. See [testing instructions](docs/TESTING.md) for browser, proxy, desktop and Android TV validation.

## Remote access

Publish the server behind your own HTTPS reverse proxy. Set `THELXINOE_PUBLIC_URL` to its origin and `THELXINOE_TRUSTED_PROXIES` to the exact proxy IPs/CIDRs. Proxy the Host, X-Forwarded-Host, X-Forwarded-Proto, and X-Forwarded-For headers and support WebSocket upgrades. Untrusted clients cannot override the request scheme or address with forwarded headers. Same-origin web sessions use HttpOnly/SameSite=Strict cookies, with Secure when reached through trusted HTTPS.

The controller is intentionally unavailable outside its Unix socket. Do not publish the Docker daemon or mount it into the server.

The web app supports installation as a Chromium PWA, mobile layouts, an offline reconnect screen and automatic remote video quality. See [remote access](docs/REMOTE.md) for proxy, CORS and cache behavior.

## YouTube

Administrators configure a Google Web application in Settings, then each person connects their own YouTube account. The server owns OAuth, refresh, subscription/feed synchronization and the shared quota budget. Watchlists, pins and watched flags are private to each user. Public VOD/live streaming and retained downloads work in the browser and Windows MPV. Twitch uses per-user device authorization; Kick supports privately tracked public channels. See [provider setup and limits](docs/ONLINE.md).

## Administration and updates

Settings includes users/devices, service health, jobs, notifications, diagnostics, encrypted backups and recovery. The managed stack supports Radarr, Sonarr, Lidarr, Bazarr, Prowlarr and NZBGet, with guarded acquisition, retention and service updates. See [operations](docs/OPERATIONS.md), [managed services](docs/MANAGED-STACK.md) and [retention](docs/RETENTION.md).

Signed product releases coordinate the server, controller, web and Windows application. Notify is the default; Automatic requires a maintenance window, idle state and tested recovery. Keep the deployment directory and retained images with backups. See [publishing and offline recovery](docs/RELEASES.md).

## TV clients

Wholphin and Jellyfin Android TV can connect to the server's HTTP(S) address using a Thelxinoe account or Quick Connect approved from web Settings. The adapter exposes the local Movies, Shows and Music catalog. See [tested client versions and limitations](docs/JELLYFIN.md).

For LAN auto-discovery, set `THELXINOE_DISCOVERY_URL` to the origin reachable by TVs, for example `http://192.168.1.20:8484`, and expose the HTTP listener using `THELXINOE_LISTEN`. Compose publishes UDP 7359 for discovery. A configured `THELXINOE_PUBLIC_URL` is used when the discovery URL is omitted; with neither configured, discovery is disabled. Set `THELXINOE_DISCOVERY=false` to disable it explicitly. Routed networks and emulator NAT may require entering the server address manually.
