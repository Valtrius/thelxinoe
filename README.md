# Thelxinoe

A self-hosted media server with a shared Svelte web and Windows Tauri application.

Implementation follows [ROADMAP.md](ROADMAP.md). See [implementation status](docs/STATUS.md) for verified behavior and remaining work. This is a development build, not a v1 release.

## Development

Requires Rust 1.96+, Node 24+, FFmpeg/FFprobe, and Docker with Linux containers.

```powershell
npm ci
npm run build
cargo run -p thelxinoe-server
```

Open http://127.0.0.1:8484. First-run setup requires the code in `.local/server/secrets/setup-token`. Development data remains in `.local`; it never uses YouTwitch's application directories. For frontend hot reload, run `npm run dev` in another terminal. Vite proxies the API and WebSocket to port 8484.

```sh
docker compose up --build -d
docker compose exec server cat /var/lib/thelxinoe/secrets/setup-token
```

For Linux bind mounts, create the server, cache, and backup directories before startup and give them to uid/gid `10001:10001`. Configure persistent paths using `.env.example`. The Docker daemon is mounted only into the controller, which has no TCP network. The server connects over a private Unix socket.

The first account is an administrator. Additional accounts require an administrator. Passwords require at least 12 characters. Provider secrets use AES-256-GCM with a persistent master key outside SQLite. Losing that key loses the ability to decrypt provider credentials.

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

## TV clients

Wholphin and Jellyfin Android TV can connect to the server's HTTP(S) address using a Thelxinoe account or Quick Connect approved from web Settings. The adapter exposes the local Movies, Shows and Music catalog. See [tested client versions and limitations](docs/JELLYFIN.md).

For LAN auto-discovery, set `THELXINOE_DISCOVERY_URL` to the origin reachable by TVs, for example `http://192.168.1.20:8484`, and expose the HTTP listener using `THELXINOE_LISTEN`. Compose publishes UDP 7359 for discovery. A configured `THELXINOE_PUBLIC_URL` is used when the discovery URL is omitted; with neither configured, discovery is disabled. Set `THELXINOE_DISCOVERY=false` to disable it explicitly. Routed networks and emulator NAT may require entering the server address manually.
