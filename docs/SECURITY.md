# Identity, credentials and API boundaries

## Secrets

The server generates one master encryption key on first boot. It stores the key in a restricted file under the persistent state tree, separate from SQLite.

OAuth tokens, API keys, provider client secrets, invitation secrets, and similar values are encrypted before SQLite storage.

Portable backups that contain enough material to restore credentials are encrypted with an administrator-supplied backup passphrase. Internal rollback snapshots rely on host filesystem permissions.

## HTTP and realtime APIs

First-party clients use a versioned HTTP API for commands and queries and a WebSocket channel for realtime events.

Typical first-party API groups:

```text
/api/v1/auth
/api/v1/users
/api/v1/catalog
/api/v1/movies
/api/v1/shows
/api/v1/music
/api/v1/youtube
/api/v1/twitch
/api/v1/kick
/api/v1/playback
/api/v1/requests
/api/v1/retention
/api/v1/integrations
/api/v1/admin
/api/v1/events        WebSocket upgrade
```

The v1 API is a first-party contract. It is structured and versioned cleanly, but external automation compatibility is not promised yet.

First-party playback endpoints issue short-lived playback grants. First-party stream, HLS, subtitle, and image URLs must not expose long-lived account or device tokens.

Jellyfin compatibility is a separate credential-transport boundary. It accepts revocable compatibility device tokens through the standard Jellyfin transports required by the tested clients, including URL query transport when a client constructs a media or image URL that way. Compatibility tokens map to the authenticated Thelxinoe user and are accepted only by the covered Jellyfin media-client surface. They are not browser session cookies and are not reusable as first-party API credentials.

When Thelxinoe generates a Jellyfin-compatible playback URL itself it should use a short-lived playback grant where the target client can consume it, but compatibility must not require an unmodified Jellyfin client to understand a Thelxinoe-specific grant protocol. HTTP access logs, diagnostics, and audit payloads redact playback grants, `ApiKey`, and equivalent query credentials.

## Authentication and authorization

The first account becomes an administrator. Administrators create or invite later users. Public registration is disabled.

V1 exposes two roles in the UI:

- `admin`
- `user`

Handlers authorize named capabilities internally instead of scattering role checks. This allows later granular roles without rewriting every endpoint.

Web sessions use secure HTTP-only cookies. Tauri and Jellyfin-compatible clients use revocable device tokens. Users can inspect and revoke their own devices. Administrators can inspect and revoke any user's devices.

Credential transport is separate from authorization. A Jellyfin compatibility token does not gain additional capabilities because a client sends it using Jellyfin conventions; every compatibility handler still resolves the same user/capability checks as the first-party domain operation it represents.

Passkeys are a future authentication method. Remote access itself is supported in v1 through an administrator-managed reverse proxy and TLS setup.
