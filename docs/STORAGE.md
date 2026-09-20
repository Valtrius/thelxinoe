# Storage and deployment paths

## Persistent storage

Host paths are deployment configuration. Compose mounts them before the server starts.

Recommended host layout:

```text
<STATE_ROOT>/
  deployment/
    compose.yaml
    compose.override.yaml
    desired-state.json
  server/
  services/
    radarr/
    sonarr/
    lidarr/
    bazarr/
    prowlarr/
    nzbget/
  backups/

<DATA_ROOT>/
  media/
    movies/
    shows/
    music/
  downloads/
```

Recommended container paths:

```text
/var/lib/thelxinoe        Thelxinoe database, state, tools, secrets
/var/cache/thelxinoe      bounded temporary/transcode/work storage
/data/media/movies
/data/media/shows
/data/media/music
/data/downloads
/backups                  optional mounted backup destination
```

Thelxinoe-created media services use the same `/data/...` namespace so their API paths match Thelxinoe paths. Existing adopted containers can keep their mounts and use explicit path reconciliation.

SQLite runs in WAL mode on local appdata storage. It must not live on the media SMB/NFS share.

The deployment descriptor and generated Compose override are intentionally independent of the server SQLite database so the controller and host-side recovery path can restore or recreate the current first-party containers when the server database is unavailable or incompatible with the running image. The override is updated only when a new deployment generation commits, so an interrupted update leaves the previous accepted generation as the Compose recovery target.
