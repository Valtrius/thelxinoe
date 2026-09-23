# Deployment and recovery

## Storage and services

Copy [`.env.example`](../.env.example) to `.env` and choose paths before starting [Compose](../compose.yaml). On Linux, pre-create the server, cache, media and backup directories with access for UID/GID `10001:10001`. Keep SQLite on local storage, and state/backups outside media. The controller deployment directory needs a Linux filesystem with atomic rename and symlinks; use a Linux volume on Docker Desktop.

Every media service must bind the same absolute host directory, writable, at `/media`:

```text
<MEDIA_ROOT>/movies       Radarr: /media/movies
<MEDIA_ROOT>/tv           Sonarr: /media/tv
<MEDIA_ROOT>/music        Lidarr: /media/music
<MEDIA_ROOT>/downloads    Download clients: /media/downloads
```

Keep each service's separate `/config` mount. Prowlarr needs no media mount. Separate child mounts and path translations are unsupported. Update existing manager records and download locations when changing paths; changing mounts alone does not update them. Hardlinks require a common filesystem.

Existing services must share a Docker network with the server. For an existing network, add this `compose.override.yaml`:

```yaml
networks:
  default:
    external: true
    name: media_network
```

Connect services in Settings → Media services and save acquisition profiles. Connecting their APIs leaves lifecycle ownership with the original Compose project. To transfer ownership, use **Review ownership transfer**: disable the service in its old Compose file and external updaters before confirming. Thelxinoe stops it and copies its appdata; never restart the retained original alongside its replacement. Avoid `--remove-orphans` while retaining the original for recovery. Interrupted transfers offer **Retry setup**, **Reconcile** or **Restore original service**. Implementation: [adoption](../apps/docker-controller/src/adoption.rs), [storage contract](../apps/docker-controller/src/contract.rs).

## Update selection and policies

New managed media services use the immutable image digests in the controller's [service templates](../apps/docker-controller/src/templates.rs). Update discovery resolves each template repository's `latest` tag to a digest. The code calls that channel stable; it does not rank version numbers or certify a newly discovered image. Preparation resolves the tag again, pins that digest, and checks compatibility against a copy of the service's appdata before installation. See [discovery](../apps/docker-controller/src/stack.rs) and [preflight](../apps/docker-controller/src/updates.rs).

The former “Stable candidate” label displayed the discovered digest even when it matched the installed image. Settings now show **Up to date** for that match and **Available image** when they differ. Image changes can include container rebuilds without an application version change.

Both **Notify** and **Automatic** check every six hours in the background, with the first check due after initial setup. Notify leaves preparation and installation to an administrator. Automatic queues discovered updates and waits for the server's maintenance window and idle checks before proceeding. Media services can inherit the server policy or override it. Attached services remain under their original owner's update policy. [Scheduler](../apps/server/src/managers/updates.rs).

Server updates use a signed release manifest with version and compatibility checks. They do not discover releases through a container `latest` tag. Their background checks require a configured release channel and signing public key; Notify is the default. [Server release policy](../apps/server/src/product.rs).

## HTTPS and TV access

Serve the web app and API at one HTTPS origin. Set `THELXINOE_PUBLIC_URL` to that origin and `THELXINOE_TRUSTED_PROXIES` to the proxy's exact IPs/CIDRs. Forward Host, X-Forwarded-Host, X-Forwarded-Proto and X-Forwarded-For; support WebSocket upgrades, Range requests and long streams. [Caddy fixture](../tests/Caddyfile). HTTP localhost works for development. Ordinary web/PWA use needs no CORS; separate browser origins require explicit `THELXINOE_CORS_ORIGINS` and remain subject to cookie restrictions.

Only the controller gets the Docker socket; keep its private Unix socket and Docker daemon unexposed.

TV clients use the server HTTP(S) address and a Thelxinoe account, or Quick Connect approved in web Settings. For discovery, set `THELXINOE_DISCOVERY_URL` to a TV-reachable origin and bind `THELXINOE_LISTEN` to the LAN address. Compose publishes UDP 7359. Discovery falls back to the public URL; neither URL means disabled. Routed networks may need manual entry. [Discovery code](../apps/server/src/jellyfin/discovery.rs).

## Backups and offline recovery

Use Settings → Backups with a passphrase of at least 16 bytes. Backups briefly stop the server and managed services and include their state, credentials and deployment descriptor; media/cache are excluded. Copy encrypted `.age` archives off-host and retain the passphrase separately. Preserve UUID filenames for import. External replication and backup retention are your responsibility.

Keep the server database **and its encryption key**, controller deployment directory and retained images. Losing the key makes provider secrets unreadable. Cross-host restore also requires the original mount/network layout. Restore can reinstate deleted accounts; it cannot undo media changes or external service activity. [Backup implementation](../apps/docker-controller/src/backups.rs).

If the HTTP server is unavailable, run these from the accepted controller container (replace `OPERATION_UUID` with the selected operation):

```sh
curl --unix-socket /run/thelxinoe/controller.sock http://localhost/stack/product
curl --unix-socket /run/thelxinoe/controller.sock \
  -H 'Content-Type: application/json' -d '{"confirm":true}' \
  http://localhost/stack/product/OPERATION_UUID/recover
```

Recreate from the generated deployment directory using both `compose.yaml` and `compose.override.yaml`; the override pins accepted images. Preserve recovery images until restoration is verified. [Recovery implementation](../apps/docker-controller/src/product.rs).

The fresh [schema](../crates/database/schema.sql) rejects earlier development databases and backups. Prefer `npm run dev:fresh`. To discard an old development database, stop its server, remove only its `thelxinoe.sqlite3`, `thelxinoe.sqlite3-wal` and `thelxinoe.sqlite3-shm`, then restart and complete setup. This loses accounts, connections, catalog state and preferences; media is separate.

## Publishing

No public release host or registry is configured. Product updates use a signed HTTPS envelope configured by `THELXINOE_RELEASE_URL`. Notify is the default; Automatic requires an idle maintenance window and successful recovery checks.

1. Change `[workspace.package].version` in [Cargo.toml](../Cargo.toml), then run `npm run version:sync` and the [release checks](TESTING.md#release-checks). Schema source/target must match this baseline; incompatible schema upgrades require an upgrade design.
2. Build and publish Linux server/controller images, retaining their manifest and platform config digests. Build Windows with `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; a Tauri override must enable `bundle.createUpdaterArtifacts` and set `plugins.updater.pubkey`.
3. Prepare a manifest matching the [release types and validation](../crates/releases/src/lib.rs), with the final HTTPS installer URL. Keep the Tauri signing key separate from the manifest Ed25519 key. The committed [publisher public key](../releases/release.pub) must match the private manifest key.
4. Assemble and sign:

```sh
node scripts/assemble-release.mjs draft.json installer.exe installer.exe.sig updater.pub manifest.json
node scripts/sign-release.mjs manifest.json private-manifest-key.pem latest.json
```

Publish the installer, adjacent `.sig`, generated `windows-x64.json` beside the installer URL, and signed `latest.json`. Back up signing keys privately; never put them on the release host.
