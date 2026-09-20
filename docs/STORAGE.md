# Storage and deployment paths

## One shared media directory

Thelxinoe and every connected media service must bind the **same host directory** at `/media`, writable, with this layout:

```text
<MEDIA_ROOT>/
  movies/       Radarr root: /media/movies
  tv/           Sonarr root: /media/tv
  music/        Lidarr root: /media/music
  downloads/    download-client storage
```

Set `THELXINOE_MEDIA_ROOT` in `.env`; its default is `./.local/media`. For example:

```dotenv
THELXINOE_MEDIA_ROOT=/volume1/media
```

Every media container uses this bind, alongside its own configuration mount:

```yaml
volumes:
  - ${THELXINOE_MEDIA_ROOT}:/media
```

Use the same absolute host path in each Compose project. Individual mounts at `/media/movies`, `/media/tv`, or anywhere below `/media` are rejected. Mounts at `/movies`, `/tv`, or `/downloads` do not satisfy the requirement. Thelxinoe does not translate paths or follow symlink aliases to other storage. Prowlarr does not access media and needs no media mount.

A single common mount supports hardlinks and atomic moves when downloads and libraries share a filesystem. Keeping the required folder names does not make hardlinks work across underlying filesystems. See the [Servarr Docker guide](https://github.com/Servarr/Wiki/blob/master/docker-guide.md#consistent-and-well-planned-paths).

## Connecting existing services on the same NAS

1. Put movies, TV, music, and downloads in the folders above. If they already have this layout under a common parent, files can stay where they are.
2. Replace the existing services' separate media mounts with the same `${THELXINOE_MEDIA_ROOT}:/media` bind used by Thelxinoe. Keep each service's `/config` mount.
3. In Radarr, Sonarr, and Lidarr, configure the corresponding root listed above. Update existing movie, series, or artist paths in the application's bulk editor. Choose not to move files if you already moved them or only changed the container path. Update download-client locations to `/media/downloads` and adjust existing jobs where necessary. Changing a Docker mount does not rewrite application records.
4. Put the server and external services on one shared Docker network. To use an existing network, add the override below.
5. In Thelxinoe, add the canonical library roots, connect each existing container with its API key and internal port, then save acquisition profiles. Onboarding verifies the shared bind source and configured library roots. Errors identify the required layout.

```yaml
# compose.override.yaml next to Thelxinoe's compose.yaml
networks:
  default:
    external: true
    name: media_network
```

Use the actual network name from the existing services; another Compose project may prefix it. The controller stays on `network_mode: none` and communicates through its private Unix socket.

An API connection leaves Docker lifecycle ownership with the original Compose project. Adoption is separate and requires removing competing orchestrator ownership. Neither operation moves library files automatically.

The server and newly managed services use UID/GID `10001:10001`. Create the host directories and grant appropriate NAS permissions or ACLs. External services may use another user if their permissions allow access to the same files.

## Managed services and state

Created media services receive the same single `/media` bind. Sonarr uses `/media/tv`; NZBGet uses `/media/downloads/completed` and `/media/downloads/intermediate`. Each service also receives its own appdata directory at `/config`:

```text
<STATE_ROOT>/
  server/                         Thelxinoe database, tools and secrets
  cache/                          temporary/transcode files
  deployment/
    compose.yaml
    compose.override.yaml
    desired-state.json
    services/<installation-id>/appdata/
<BACKUP_ROOT>/                     encrypted backups
```

Choose storage before creating the stack. Changing Compose mounts does not automatically reconfigure existing managed containers; drift checks block using stale storage evidence. Updates and recovery retain the accepted mount source. Compatibility checks use disposable scratch media.

Keep state and backups outside the media directory. SQLite must use local appdata rather than an SMB/NFS share. The controller's deployment descriptor and recovery Compose file are independent of the server database. See [managed services](MANAGED-STACK.md) for lifecycle and recovery details.

## Validation

The isolated Docker proof connects a real external Radarr and installs managed Sonarr with the same `/media` bind. It verifies shared file contents, rejects a noncanonical Radarr root, creates `/media/tv`, checks writes through Sonarr, and verifies the recovery mount.

```sh
docker build --target server -t thelxinoe-media-server:local .
docker build --target controller -t thelxinoe-media-controller:local .
node scripts/test-media-mounts.mjs
```

Use a fresh `.local/media-mount-test` directory. The fixture is exercised with Docker Desktop on Windows and removes its containers, network and Docker volumes in `finally`. Sanitized results remain in that directory. Unit tests reject mismatched host sources, child mounts, read-only mounts, named volumes and unsupported library roots.
