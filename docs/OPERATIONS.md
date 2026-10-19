# Deployment and recovery

## Storage and services

Copy [`.env.example`](../.env.example) to `.env` and choose paths before starting [Compose](../compose.yaml). Set `THELXINOE_UID` and `THELXINOE_GID` to your host media account's numeric IDs (`id -u` and `id -g`); both default to `10001` and must be nonzero. On Linux, pre-create the server, cache, media and backup directories with access for those IDs. Keep SQLite on local storage, and state/backups outside media. The release Compose file keeps the controller's deployment record in a Docker volume; building from source keeps it under `THELXINOE_STATE_ROOT/deployment`, which needs a Linux filesystem with atomic rename and symlinks (use a Linux volume on Docker Desktop).

The server and newly installed media services use these IDs. The controller runs as root with the configured group and sets access on its private socket directory. Owned services retain their saved identities through updates and recovery; a transfer gives the copied configuration to the deployment's identity and leaves the original's files unchanged. Choose IDs before initializing the deployment; changing `.env` does not change existing file ownership or saved deployment/service identities.

After the first start, the controller adopts containers that Compose recreates, after `docker compose pull`, a rebuild or a changed `.env`/override, as the next generation of the same deployment, so installed and transferred services stay registered. A recreated controller adopts them when it starts; a running controller adopts a server recreated on its own within a minute. Adoption refuses a different deployment storage, a server and controller from different Thelxinoe versions, a server on more than one network, and a changed network or media folder that Thelxinoe-installed services still use; it waits until an interrupted update, backup or restore has finished. The reason appears in `docker compose logs controller`. [Adoption](../apps/docker-controller/src/product.rs).

Snapshots, updates, transfers and removals run in short-lived worker containers with memory, CPU and process limits. Engines whose kernel lacks CPU bandwidth control or the pids cgroup, such as Synology Container Manager, drop those two limits and the controller logs Docker's warning; memory limits still apply. When Docker refuses a request, the error names the operation and Docker's reason, with setting values removed; `docker compose logs controller` has the full message.

Fresh installations share the configured media directory at `/media`:

```text
<MEDIA_ROOT>/movies       Radarr: /media/movies
<MEDIA_ROOT>/tv           Sonarr: /media/tv
<MEDIA_ROOT>/music        Lidarr: /media/music
<MEDIA_ROOT>/downloads    Download clients: /media/downloads
```

Existing services retain their mounts and native paths. Keep the server's writable `/media` bind mount; additional bind mounts or shared named volumes beneath `/media` support existing storage layouts. Fresh media installations require a single shared mount without child mounts. Playback requires read access. Hardlinks require a common filesystem.

Existing services must share a user-defined Docker network with the server; Docker's default `bridge` network cannot resolve container names, and services using another container's network (such as a VPN container) are not supported yet. Connection errors name both sides' networks and the `docker network connect` command to run. For an existing network, add this `compose.override.yaml`:

```yaml
networks:
  default:
    external: true
    name: media_network
```

Install or connect services in Settings → Media services. A Connected service is validation-only: Thelxinoe checks its container identity, health and API key but never writes to it or manages it, and every operation (requests, downloads, sync, links, backups, native access) requires an Owned service. One database flag, maintained in the same transaction as ownership and its holds, gates all of that work; [ownership](../apps/server/src/managers/ownership.rs). When the current owner recreates a Connected container, Thelxinoe follows the replacement only if its name, mounts, API identity and stored credentials all match; [recreation](../apps/server/src/managers/recreation.rs). Otherwise, choose the container in **Edit connection**.

Follow the [ownership steps](../README.md#bring-existing-services-to-thelxinoe) to take ownership. The controller journals every transfer step under its secret store and resumes after a restart; [transfer](../apps/docker-controller/src/transfer.rs). Before the replacement starts, any failure restores the original. Afterwards, the service stays Owned under an operational hold until its required checks pass or you choose a recovery, and the original stays stopped with its configuration untouched. The server follows the controller's journal: a lost or failed reply is observed again rather than treated as a failure, and an approved recovery is journaled phase by phase and resumes after a restart. Completion releases the controller's hold first and then the server's, so both sides stay held until they agree. A recreated container of any Owned service, installed or transferred, is held until its API is reverified. Fresh installs also hold normal operations until their setup completes. Fresh Radarr/Sonarr installs receive request defaults; existing services require an explicit choice of existing root and quality profile.

## Discovery and requests

Install **Seerr** in Media services. Home provides discovery, search, movie/show details, season selection, and the request queue. Seerr supplies TMDB metadata and uses connected Radarr/Sonarr services for availability. Manually imported Thelxinoe catalog entries are excluded. Library pages are for browsing and playback.

The server manages Seerr API credentials and maps each signed-in Thelxinoe user to a local Seerr account. Regular users request media and see their own queue; administrators approve, decline, or retry requests. The existing per-user automatic approval setting also applies. Seerr's local and media-server sign-in are disabled for managed installations.

Upstream Seerr requires an initial media-server administrator before it exposes local-user creation. Managed setup performs that handshake against a temporary empty endpoint, authenticated with a random token that expires after two minutes and is revoked as soon as setup finishes. It exposes no catalog or Thelxinoe login. Setup then clears the media-server type; subsequent availability scans come from Arr. The update preflight verifies the Seerr API and existing owner against copied state; first-run initialization also needs checking when qualifying new upstream releases. [Adapter](../apps/server/src/managers/seerr.rs).

Application connections are managed only after explicit permission, except between fresh Thelxinoe installations. Requests synchronize their selected manager before submission. **Refresh connections** also runs availability scans.

Prowlarr's **Add indexer** form loads its supported definitions, fields, profiles, and address choices from the API. It supports connection testing and image challenges. Managed Prowlarr hosts are configured from the internal service address and configured public/native URLs. Indexer-to-manager connections still use the separate service connection controls.

## Public YouTube network

Trailers and other public YouTube videos are extracted without accounts. YouTube can block a server's address with a "Sign in to confirm you're not a bot" check, often for one address family only. Settings → Provider applications → **Public video network** selects IPv4, IPv6, or **Automatic**, which starts with IPv4, switches to IPv6 when IPv4 is blocked, and keeps the family that last worked. YouTube signs media addresses for the extracting IP address, so extraction, downloads, the relay, and ffmpeg all use the chosen family; ffmpeg reaches YouTube's media CDN through a loopback proxy pinned to that family. [Selection](../apps/server/src/online/network.rs) and [proxy](../apps/server/src/online/egress.rs).

Recorded videos play YouTube's original files without conversion. Browsers and Jellyfin clients that accept fragmented MP4 HLS receive a byte-range playlist built from each file's own fragment index, so they seek without a server round trip; the desktop player reads the video and audio files directly. Both read through the server's relay, which requests bounded 10 MiB ranges because YouTube throttles open-ended reads to about twice the bitrate. Clients without fragmented MP4 HLS, and live streams, use the FFmpeg pipeline. [Delivery](../apps/server/src/online/streams.rs) and [relay](../apps/server/src/online/relay.rs).

Set `THELXINOE_IPV6=true` in `.env` to enable IPv6 on the server's network (on by default when building from source). Docker Desktop 4.42 or later and Docker Engine 27 or later assign it a private IPv6 subnet automatically; the host still needs IPv6 internet access. An external network keeps its own settings, so create it with `docker network create --ipv6`.

## Update selection and policies

New managed services resolve the curated repository's `latest` tag and install that immutable digest. Update discovery uses the same channel, so a new installation is current unless the upstream image changes afterward. The allowed repositories and API ports are defined in the controller's [service templates](../apps/docker-controller/src/templates.rs). The code calls that channel stable; it does not rank version numbers or certify a newly discovered image. Preparation resolves the tag again, pins that digest, and checks compatibility against a copy of the service's appdata before installation. See [discovery](../apps/docker-controller/src/stack.rs) and [preflight](../apps/docker-controller/src/updates.rs).

The former “Stable candidate” label displayed the discovered digest even when it matched the installed image. Settings now show **Up to date** for that match and **Available image** when they differ. Image changes can include container rebuilds without an application version change.

Both **Notify** and **Automatic** check every six hours in the background, with the first check due after initial setup. Notify leaves preparation and installation to an administrator. Automatic queues discovered updates and waits for the server's maintenance window and idle checks before proceeding. Media services can inherit the server policy or override it. Attached services remain under their original owner's update policy. [Scheduler](../apps/server/src/managers/updates.rs).

Server updates use a signed release manifest with version and compatibility checks; GitHub Releases is the default channel and the signing public key is included in the images. **Automatic** is the default server policy: it installs a new release in the nightly maintenance window, 03:00 to 05:00 in the server's timezone, which setup takes from the administrator's browser. An automatic attempt that cannot prepare, for example because a download failed, is retried after 15 minutes, then up to every 6 hours; a release that fails validation or activation is not retried automatically. The release `compose.yaml` follows the `latest` image tags, so `docker compose pull && docker compose up -d` installs the newest release by hand at any time. [Server release policy](../apps/server/src/product.rs).

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

Use Settings → Backups with a passphrase of at least 16 bytes. Backups briefly stop the server and managed services and include their state, credentials and deployment descriptor; media/cache are excluded. A service held by an ownership transfer or recovery, or whose container changed or is missing, keeps running and is listed as not included. Server snapshots leave out media-tool installs in progress, which the server discards at startup. Copy encrypted `.age` archives off-host and retain the passphrase separately. Preserve UUID filenames for import. External replication is your responsibility.

Restore lets you choose components. By default it restores the server and every Owned service in the archive; only chosen components stop. A retained original container from a transfer is never part of a backup or restore. Each service's backup copy is staged beside its current configuration and then swapped in, so its storage needs free space for a second copy, and an interrupted swap is undone on the next attempt. Copies may take as long as they keep progressing; one that makes no progress for 10 minutes fails.

**Manual** is the default backup policy. Choose **Automatic** in Settings → Backups to run one backup per day when the server maintenance window (Settings → Server) opens, before automatic server, service and tool updates in that window. It needs a passphrase saved in Settings → Backups. The server stores it encrypted with its credential key, and saving it requires a recent sign-in. Until a passphrase is saved, nothing runs and an attention item asks for one. The backup waits for active playback, running jobs and media operations to finish. If the server is not idle within 30 minutes of the window opening, that day's backup is skipped. Updates continue after a failed or skipped backup and the outcome is shown as an attention item. After each successful automatic backup, the controller keeps the configured number of newest automatic archives (default 7) and deletes older automatic archives and earlier failed attempts. Manual and imported archives are never deleted. Replacing the saved passphrase only affects later archives. Switching back to Manual stops scheduling and rotation. [Scheduler](../apps/server/src/backups.rs).

Keep the server database **and its encryption key**, controller deployment directory (including its registry key) and retained images. Losing a key makes its encrypted credentials and registry unreadable. Cross-host restore requires the original mount/network layout. Owned services are tied to the deployment's Docker engine; a transfer retains its stopped original only on that engine. Restore can reinstate deleted accounts; it cannot undo media changes or external service activity. [Backup implementation](../apps/docker-controller/src/backups.rs).

If the HTTP server is unavailable, run these from the accepted controller container (replace `OPERATION_UUID` with the selected operation):

```sh
curl --unix-socket /run/thelxinoe/controller.sock http://localhost/stack/product
curl --unix-socket /run/thelxinoe/controller.sock \
  -H 'Content-Type: application/json' -d '{"confirm":true}' \
  http://localhost/stack/product/OPERATION_UUID/recover
```

To reinstall, run `docker compose pull && docker compose up -d` from the folder with your `compose.yaml` and `.env`; the controller adopts the new containers. Preserve recovery images until restoration is verified. [Recovery implementation](../apps/docker-controller/src/product.rs).

Each release upgrades databases from earlier released [schemas](../crates/database/schemas) in place. Development databases from before the first release are rejected. Prefer `pnpm run dev:fresh`. To discard an old development database, stop its server, remove only its `thelxinoe.sqlite3`, `thelxinoe.sqlite3-wal` and `thelxinoe.sqlite3-shm`, then restart and complete setup. This loses accounts, connections, catalog state and preferences; media is separate.

## Publishing

See [release workflows, signing setup and local update qualification](UPDATES.md).
GitHub Releases supplies signed Windows artifacts and publisher metadata; GHCR
supplies immutable server/controller images. Assembly creates a draft; publication
is a separate manual workflow.
