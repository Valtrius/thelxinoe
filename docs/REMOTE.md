# Web, PWA and remote access

Serve Thelxinoe and its API at one HTTPS origin. Set `THELXINOE_PUBLIC_URL` to that origin without a path, query or credentials. Set `THELXINOE_TRUSTED_PROXIES` only to the reverse proxy's addresses or dedicated network. The supplied Caddy fixture exercises this model. Do not expose the controller socket or Docker socket through the proxy.

The proxy must preserve the host and support WebSocket upgrades, streaming responses, Range requests and long media transfers. Forward scheme/host/client headers only from the trusted proxy. The server resolves client addresses from the right-hand side of the forwarded chain and ignores forwarded headers from other peers. HTTPS sessions use Secure, HttpOnly, SameSite=Strict cookies. Redirects, OAuth callbacks and generated media URLs use the configured public origin. Query credentials are excluded from diagnostics and normal application logs.

CORS is disabled by default. A separate browser client may be allowed explicitly with comma-separated `THELXINOE_CORS_ORIGINS` origins; wildcards, paths and embedded credentials are rejected. Allowed origins receive bounded methods/headers and credential support. SameSite=Strict still applies to cookies; cross-site API clients require a revocable device credential. The normal PWA needs no CORS configuration.

The desktop authenticates API requests through its native transport. Its event WebSocket uses a 30-second, single-use ticket tied to that device session. Valid device tickets also work from the desktop development page's origin without adding Vite to the CORS allowlist. Browser-session tickets still require the normal origin checks; device tickets grant no access to other endpoints and stop working when the session is revoked.

Install the app from Chromium's install menu after opening the HTTPS site. Its manifest declares standalone display and 192/512-pixel icons. Only a public connection screen is retained by the service worker. API responses, credentials, media and application bundles are never stored by it. If offline, the installed app shows a reconnect action; media playback requires the server. Background Web Push and offline media remain outside v1.

Auto video quality uses a 4 Mbps transcode default for public client addresses when the client supports HLS. LAN clients keep automatic direct-play/remux behavior. Explicit Original and bitrate choices take precedence. Music keeps its existing direct/gapless path. Correct proxy trust is necessary for address-based defaults; remote access through a private VPN is treated as LAN.

`scripts/test-pwa-remote.mjs` exercises fresh setup on a 390-pixel Chromium mobile viewport, all main sections without document overflow, installability, offline reconnect, cookie/CSRF/API-version checks, HTTPS Range and HLS requests, and the trusted remote bitrate default. Earlier proxy fixtures additionally cover event replay/reconnect, device revocation and OAuth callbacks. See [TESTING.md](TESTING.md).

## Reverse proxy and public URL

The server can operate on a LAN without a public URL.

When a public URL is configured, the server uses it for externally valid OAuth callbacks, invitations, and absolute links. Forwarded scheme, host, and client IP headers are trusted only from configured proxy addresses or networks.

The web frontend and API should share one origin. CORS is enabled only for explicit origins that need browser cross-origin access.
