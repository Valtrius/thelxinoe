# Deployment and recovery

## Storage and services

Copy [`.env.example`](../.env.example) to `.env` and choose paths before starting [Compose](../compose.yaml). Set `THELXINOE_UID` and `THELXINOE_GID` to your host media account's numeric IDs (`id -u` and `id -g`); both default to `10001` and must be nonzero. On Linux, pre-create the server, cache, media and backup directories with access for those IDs. Keep SQLite on local storage, and state/backups outside media. The controller deployment directory needs a Linux filesystem with atomic rename and symlinks; use a Linux volume on Docker Desktop.

The server and newly installed media services use these IDs. The controller runs as root with the configured group and sets access on its private socket directory. Existing services retain their saved identities through adoption, updates and recovery. Choose IDs before initializing the deployment; changing `.env` does not change existing file ownership or saved deployment/service identities.

Fresh installations share the configured media directory at `/media`:

```text
<MEDIA_ROOT>/movies       Radarr: /media/movies
<MEDIA_ROOT>/tv           Sonarr: /media/tv
<MEDIA_ROOT>/music        Lidarr: /media/music
<MEDIA_ROOT>/downloads    Download clients: /media/downloads
```

Existing services retain their mounts and native paths. Keep the server's writable `/media` bind mount; additional bind mounts or shared named volumes beneath `/media` support existing storage layouts. Fresh media installations require a single shared mount without child mounts. Playback requires read access. Hardlinks require a common filesystem.

Existing services must share a Docker network with the server. For an existing network, add this `compose.override.yaml`:

```yaml
networks:
  default:
    external: true
    name: media_network
```

Connect services in Settings → Media services. Fresh Radarr/Sonarr installs receive request defaults; existing services require an explicit choice of existing root and quality profile. API integration leaves container management with the current owner. Follow the [adoption steps](../README.md#adopt-existing-services) to transfer ownership. In-place adoption requires retiring the entire original Compose project and its updaters. Configuration stays where it is. Imported recreation and updates are currently blocked; **Release ownership** leaves the container and data intact.

## Discovery and requests

Install **Seerr** in Media services. Home provides discovery, search, movie/show details, season selection, and the request queue. Seerr supplies TMDB metadata and uses connected Radarr/Sonarr services for availability. Manually imported Thelxinoe catalog entries are excluded. Library pages are for browsing and playback.

The server manages Seerr API credentials and maps each signed-in Thelxinoe user to a local Seerr account. Regular users request media and see their own queue; administrators approve, decline, or retry requests. The existing per-user automatic approval setting also applies. Seerr's local and media-server sign-in are disabled for managed installations.

Upstream Seerr requires an initial media-server administrator before it exposes local-user creation. Managed setup performs that handshake against a temporary empty endpoint, authenticated with a random token that expires after two minutes and is revoked as soon as setup finishes. It exposes no catalog or Thelxinoe login. Setup then clears the media-server type; subsequent availability scans come from Arr. The update preflight verifies the Seerr API and existing owner against copied state; first-run initialization also needs checking when qualifying new upstream releases. [Adapter](../apps/server/src/managers/seerr.rs).

Application connections are managed only after explicit permission, except between fresh Thelxinoe installations. Requests synchronize their selected manager before submission. **Refresh connections** also runs availability scans.

Prowlarr's **Add indexer** form loads its supported definitions, fields, profiles, and address choices from the API. It supports connection testing and image challenges. Managed Prowlarr hosts are configured from the internal service address and configured public/native URLs. Indexer-to-manager connections still use the separate service connection controls.

## Update selection and policies

New managed services resolve the curated repository's `latest` tag and install that immutable digest. Update discovery uses the same channel, so a new installation is current unless the upstream image changes afterward. The allowed repositories and API ports are defined in the controller's [service templates](../apps/docker-controller/src/templates.rs). The code calls that channel stable; it does not rank version numbers or certify a newly discovered image. Preparation resolves the tag again, pins that digest, and checks compatibility against a copy of the service's appdata before installation. See [discovery](../apps/docker-controller/src/stack.rs) and [preflight](../apps/docker-controller/src/updates.rs).

The former “Stable candidate” label displayed the discovered digest even when it matched the installed image. Settings now show **Up to date** for that match and **Available image** when they differ. Image changes can include container rebuilds without an application version change.

Both **Notify** and **Automatic** check every six hours in the background, with the first check due after initial setup. Notify leaves preparation and installation to an administrator. Automatic queues discovered updates and waits for the server's maintenance window and idle checks before proceeding. Media services can inherit the server policy or override it. Attached services remain under their original owner's update policy. [Scheduler](../apps/server/src/managers/updates.rs).

Server updates use a signed release manifest with version and compatibility checks. They do not discover releases through a container `latest` tag. GitHub Releases is the default channel, the signing public key is included in the images, and Notify is the default policy. [Server release policy](../apps/server/src/product.rs).

## HTTPS and TV access

Radarr, Sonarr, Lidarr, Prowlarr and Bazarr open in a new tab through Thelxinoe's
administrator session. Fresh managed installs use `/services/<service>` as URL Base.
The services need only a private Docker connection to Thelxinoe; expose the
Thelxinoe origin through the reverse proxy. Keep service ports private when
Thelxinoe supplies their authentication.

Imported and attached containers keep their authentication and API prefix,
entered during connection. Their configuration is preserved. Proxy access works
when that prefix is nonempty and does not overlap Thelxinoe or another service.
Empty or conflicting prefixes remain usable for API integration.
Set a **Native web address** to open a service directly when proxy access is
unavailable; its own authentication applies. Proxied interfaces share Thelxinoe's browser origin.

Connected NZBGet opens at `/services/nzbget/` using the saved connection credentials.
Its private API stays at the root, including `/jsonrpc`; no URL Base change is
needed. Direct service access retains NZBGet's own authentication.

The Windows app opens a single-use browser handoff at the configured public
origin, or its connected server address when no public origin is configured.
Browser access is scoped to
that service and expires after at most eight hours or when its desktop session
is revoked. Routes, authentication and streaming are implemented in
[service access](../apps/server/src/managers/access.rs).

Serve the web app and API at one HTTPS origin. Set `THELXINOE_PUBLIC_URL` to that origin and `THELXINOE_TRUSTED_PROXIES` to the proxy's exact IPs/CIDRs. Forward Host, X-Forwarded-Host, X-Forwarded-Proto and X-Forwarded-For; support WebSocket upgrades, Range requests and long streams. [Caddy fixture](../tests/Caddyfile). HTTP localhost works for development. Ordinary web/PWA use needs no CORS; separate browser origins require explicit `THELXINOE_CORS_ORIGINS` and remain subject to cookie restrictions.

Only the controller gets the Docker socket; keep its private Unix socket and Docker daemon unexposed.

TV clients use the server HTTP(S) address and a Thelxinoe account, or Quick Connect approved in web Settings. For discovery, set `THELXINOE_DISCOVERY_URL` to a TV-reachable origin and bind `THELXINOE_LISTEN` to the LAN address. Compose publishes UDP 7359. Discovery falls back to the public URL; neither URL means disabled. Routed networks may need manual entry. [Discovery code](../apps/server/src/jellyfin/discovery.rs).

## Backups and offline recovery

Use Settings → Backups with a passphrase of at least 16 bytes. Backups briefly stop the server and managed services and include their state, credentials and deployment descriptor; media/cache are excluded. Copy encrypted `.age` archives off-host and retain the passphrase separately. Preserve UUID filenames for import. External replication and backup retention are your responsibility.

Keep the server database **and its encryption key**, controller deployment directory (including its registry key) and retained images. Losing a key makes its encrypted credentials and registry unreadable. Cross-host restore requires the original mount/network layout. Imported ownership is tied to its Docker engine and full container identity; release and re-adopt on a new engine. Restore can reinstate deleted accounts; it cannot undo media changes or external service activity. [Backup implementation](../apps/docker-controller/src/backups.rs).

If the HTTP server is unavailable, run these from the accepted controller container (replace `OPERATION_UUID` with the selected operation):

```sh
curl --unix-socket /run/thelxinoe/controller.sock http://localhost/stack/product
curl --unix-socket /run/thelxinoe/controller.sock \
  -H 'Content-Type: application/json' -d '{"confirm":true}' \
  http://localhost/stack/product/OPERATION_UUID/recover
```

Recreate from the generated deployment directory using both `compose.yaml` and `compose.override.yaml`; the override pins accepted images. Preserve recovery images until restoration is verified. [Recovery implementation](../apps/docker-controller/src/product.rs).

The fresh [schema](../crates/database/schema.sql) rejects earlier development databases and backups. Prefer `pnpm run dev:fresh`. To discard an old development database, stop its server, remove only its `thelxinoe.sqlite3`, `thelxinoe.sqlite3-wal` and `thelxinoe.sqlite3-shm`, then restart and complete setup. This loses accounts, connections, catalog state and preferences; media is separate.

## Publishing

See [release workflows, signing setup and local update qualification](UPDATES.md).
GitHub Releases supplies signed Windows artifacts and publisher metadata; GHCR
supplies immutable server/controller images. Assembly creates a draft; publication
is a separate manual workflow.
