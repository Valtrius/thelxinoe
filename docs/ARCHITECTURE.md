# System architecture

## Runtime topology

The initial deployment contains two Thelxinoe containers.

```text
                         reverse proxy / LAN
                                |
                                v
                    +-------------------------+
                    |   Thelxinoe server      |
                    |                         |
                    | HTTP API + WebSocket    |
                    | Web frontend            |
                    | Jellyfin compatibility  |
                    | SQLite                  |
                    | Media services          |
                    | Background jobs         |
                    +------------+------------+
                                 |
                                 | private Unix socket
                                 v
                    +-------------------------+
                    | Docker controller       |
                    |                         |
                    | constrained operations  |
                    | curated service specs   |
                    +------------+------------+
                                 |
                                 | Docker socket
                                 v
                    +-------------------------+
                    | Docker daemon           |
                    |                         |
                    | Radarr / Sonarr / ...   |
                    +-------------------------+
```

The main server never receives the raw Docker socket. The controller exposes only Thelxinoe-specific operations over a Unix-domain socket on a private shared runtime volume.

The controller manages only services assigned to Thelxinoe. It is not a general Docker UI.

The controller also owns a small host-persistent first-party deployment descriptor outside the server database. It records the accepted product version, immutable server and controller image digests, the first-party container configuration needed for recreation, and a monotonically increasing controller generation. The persistent deployment directory contains the Compose project used for bootstrap and recovery. The controller atomically maintains `compose.override.yaml` with the accepted immutable server/controller digests when a release generation is committed. Standard `docker compose up` from that deployment directory therefore resolves the same accepted generation as `desired-state.json` instead of the stale bootstrap images in the base Compose file. The base Compose definition is not a second long-term desired-state authority.

Controller self-handoff is fenced by the deployment generation. The old controller retains the Docker mutation lease while it creates and validates the successor. The successor may become the writer only after the old generation commits the handoff in the persistent descriptor. Recovery selects the last committed generation. Two controller generations must never both be allowed to mutate managed Docker state. The controller has writable host access only to the dedicated deployment state it owns, not to arbitrary host configuration.

## Monorepo layout

One repository and one product version cover every first-party artifact.

```text
/apps
  /server                 HTTP, provider integrations, jobs and web hosting
  /docker-controller      Constrained local Docker management and recovery
  /desktop                Tauri shell, MPV and native updates
/frontend                 Shared Svelte 5 application
/crates
  /auth                    Accounts, sessions and encrypted credentials
  /backup                  Encrypted portable archives
  /catalog                 Files, logical identities and metadata
  /core                    Shared product types and version
  /database                SQLite schema and access
  /jobs                    Persistent bounded scheduler
  /playback                Playback, progress and media delivery
  /releases                Signed manifests and compatibility checks
```

Provider, manager and compatibility adapters currently live in the server application; crate boundaries can evolve when they improve ownership. The important boundary is that Tauri, HTTP, Docker, and provider-specific code do not leak into core media rules.

## Server responsibilities

The server owns all durable product state except the media files and external service databases.

It owns:

- users, roles, capabilities, sessions, and device tokens;
- per-user progress, watched state, history, statistics, favorites, Watch Later, playlists, and requests;
- per-user YouTube/Twitch/Kick viewer connections;
- local media catalog and metadata index;
- logical media identities independent of concrete files;
- playback sessions and progress reconciliation;
- notifications and audit records;
- retention policies and pending deletions;
- integration configuration and manager bindings;
- update policy and health state;
- background jobs and their state.

The server does not rename or reorganize local media. It may delete media under the retention and admin rules already defined.

## Database access

`crates/database` owns schema initialization, connections, scheduling, backups, and shutdown. Server SQL lives under `apps/server/src/storage`, grouped by domain. The standalone auth, catalog, and jobs crates each have a `storage.rs`. Handlers and workflows call named, typed storage operations; they never submit SQL closures. Storage modules are private children of their domain modules so they can use the domain's existing types without creating dependency cycles. A Rust architecture test checks this boundary, including SQL inside macros.

Each `Database` instance owns one writer thread and two reader threads. Each thread reuses its own connection. Readers open SQLite in read-only mode and run each operation in a consistent snapshot. WAL allows them to continue while the writer commits. Each lane has a queue of 64 operations and a five-second admission timeout; overload or shutdown returns HTTP 503. `Database::open_with_options` can adjust the reader count, queue capacity, and admission timeout. SQL never runs on an async executor thread. Network requests, media probing, and password hashing stay in their existing workflows. Owned download-cache deletion retains its existing transaction fence against new playback and retained interests, so that filesystem operation can briefly occupy the writer.

A storage operation contains the entire transaction, including its validation and related tables. Playback progress, statistics, history, and grants still change together. Playback reports, watchlist mutations, and YouTube sync pages also save their event rows in that transaction; subscribers are notified after commit. Large catalog scans retain their existing atomic commit, so they can delay other writes while readers continue. The runtime does not automatically retry writes.

Accepted operations finish even if their caller disconnects or cancels. Multi-statement writes use explicit transactions; errors or panics roll those transactions back. A worker also rolls back any transaction accidentally left open before reusing its connection. Server shutdown stops background producers, drains accepted database operations, and joins the threads. The database has no public connection escape hatch or generic `call` method.

The admin dashboard and diagnostics expose separate read/write queue depths, active operations, completions, failures, and cumulative wait/execution microseconds. Debug logs include the storage operation name and both timings. These measurements distinguish queue contention from expensive SQL; adding workers does not itself promise faster queries.

## Background jobs

V1 uses one main server process with a persistent internal job scheduler and queue.

Jobs cover scans, metadata refresh, platform sync, downloads, segment analysis, retention, backups, update checks, and similar work.

Jobs that can mutate durable state or external systems are restart-safe and idempotent at their command boundary. A crash may cause a job to be retried, so the durable job record stores enough identity/version information to revalidate the target before repeating an external side effect. Destructive media jobs additionally use the media-operation coordinator defined by the retention model.

Expensive work runs with bounded concurrency. FFmpeg, yt-dlp, Streamlink, and analysis tools run as managed child processes where appropriate.

Dedicated worker containers are deferred until measured load justifies them.
