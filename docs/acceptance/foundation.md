# Foundation and identity acceptance

These are the accepted v1 delivery criteria. Current results and supported limits are in [implementation status](../STATUS.md) and the [release checklist](../RELEASE_CHECKLIST.md).

## Phase 0. Repository and build foundation

Create the monorepo and shared versioning before porting product code.

Deliver:

- Cargo workspace;
- Svelte 5 frontend workspace;
- Tauri 2 desktop shell;
- Rust server binary;
- Rust Docker-controller binary;
- shared product-version source used by all artifacts;
- Dockerfiles and minimal Compose setup;
- CI jobs for Rust format, Clippy, tests, frontend format/lint/check/test/build, container builds, and Windows Tauri build;
- development configuration and local sandbox data paths.

Exit condition:

- `docker compose up` starts a server and controller;
- the browser shows a minimal frontend served by the server;
- Tauri opens the same frontend against the server;
- the controller is reachable only over the private Unix socket.

## Phase 1. Server identity, database, and first-run setup

Build the minimum durable server.

Deliver:

- SQLite migration framework in WAL mode;
- first-boot master-key generation;
- encrypted secret storage abstraction;
- user/account schema;
- password hashing;
- web sessions and revocable device tokens;
- admin/user capability model;
- first-run admin creation;
- server timezone;
- public-base-URL and trusted-proxy configuration;
- representative HTTPS reverse-proxy authentication tests covering forwarded scheme/host/client address and secure-cookie behavior;
- server health endpoint;
- frontend login/setup flow;
- device/session management.

Exit condition:

- a fresh Compose deployment can create the first admin, log in from browser and Tauri, restart without losing state, revoke a device session, and complete the same browser login flow through the supported reverse-proxy model.

## Phase 2. First-party API and realtime transport

Establish the client/server contract before porting feature UIs.

Deliver:

- `/api/v1` routing structure;
- normalized error model;
- WebSocket event channel;
- frontend backend-client abstraction replacing direct Tauri `invoke` assumptions;
- event reconnect/resubscribe behavior;
- authentication-principal abstraction separated from credential transport;
- short-lived playback-grant primitive for first-party clients;
- minimal persistent internal job queue with restart recovery, bounded execution, and idempotent command boundaries for durable/background work;
- WebSocket reconnect/resubscribe tests through a representative HTTPS reverse proxy.

Exit condition:

- the same Svelte frontend works in browser and Tauri without feature code knowing which shell hosts it;
- server events reach both clients after reconnects, including through the supported reverse-proxy path;
- a persisted test job survives a server restart and resumes without duplicating its completed side effect.
