# Product scope

## Product boundary

Thelxinoe is a self-hosted media server with first-party web and Tauri clients. It covers local Movies, Shows, and Music plus YouTube, Twitch, and Kick.

The filesystem decides which local media exists. Radarr, Sonarr, Lidarr, Bazarr, Prowlarr, and NZBGet can populate or manage that media, but Thelxinoe can also index files copied manually. Local Movies, Shows and Music stay local and self-hosted. External providers besides YouTube, Twitch and Kick are outside the current product scope.

The first supported server platform is Linux x86-64. The first supported Tauri platform is Windows x64. The responsive web client is also an installable PWA. Jellyfin media clients can connect to Thelxinoe for Movies, Shows, and Music. Wholphin is a required compatibility target.

## V1 exclusions

The following are deliberately outside v1:

- hardware transcoding;
- Linux/macOS Tauri builds;
- Linux ARM64 server support claim;
- remote API-only `*arr` instances;
- passkeys;
- external automation API guarantee;
- external backup providers;
- offline client media;
- background Web Push;
- Chromecast, DLNA, AirPlay;
- watch parties;
- first-party cross-device remote control;
- collaborative playlist editing;
- external music providers;
- audiobooks, podcasts, Photos, Live TV, IPTV, EPG, DVR;
- full Jellyfin server compatibility;
- unstable managed-service release channels.

## Product requirements

Thelxinoe is a new program derived in part from YouTwitch, not a new version of YouTwitch.

The current direction is:

- Run the main backend in Docker.
- Provide a web frontend and a Tauri desktop frontend with the same core UI.
- Treat Android TV playback through Wholphin as a first-version bonus target. Wholphin is a Jellyfin client, so this means considering a compatible Jellyfin API subset rather than building a separate TV UI first.
- Reuse suitable parts of the existing YouTwitch frontend and backend instead of rewriting them without reason.
- Keep YouTube, Twitch, and Kick capabilities that still make sense in the new product.
- Add music playback.
- Add films and TV shows playback.
- Add users and login.
- Use independent user accounts with shared media libraries by default.
- Keep watched state, playback position, history, statistics, watchlists, playlists, favorites, UI preferences, linked viewer accounts, and media requests per user. Keep the physical media library and server integrations shared.
- Create the first account as administrator. Administrators create or invite later users. Start with `admin` and `user` roles and no public registration.
- Configure Google/Twitch/Kick application credentials once at server level while each Thelxinoe user links their own viewer account and tokens.
- Design the server so it can be exposed remotely behind an administrator-managed reverse proxy and TLS setup.
- Integrate with Radarr, Sonarr, Lidarr, Bazarr, Prowlarr, and NZBGet through their APIs.
- Manage the Docker instances for the media stack, including their lifecycle and update cycle. Docker management is a product responsibility rather than an optional convenience.
- In the first version, manage those services as local Docker containers rather than supporting API-only remote instances. Support installing new managed instances and adopting compatible existing containers.
- Validate a candidate media-stack image against the API contract Thelxinoe actually uses before allowing an automatic update. Run preflight candidates against a consistent disposable appdata snapshot inside an isolated test network that cannot reach live media-stack services, the Docker daemon, host services, or arbitrary outbound targets. Use disposable or controlled test dependencies where the contract requires another service. Back up persistent service data from a consistent recovery point before live replacement.
- Manage only Thelxinoe's media-stack containers, not arbitrary Docker workloads on the host. Thelxinoe owns Docker-level configuration for managed services, while each service remains the authority for its application-level settings.
- Keep Home, Movies, Shows, Music, YouTube, Twitch, and Kick as first-class sections. Home may mix useful content from the other sections.
- Use software FFmpeg transcoding in v1 and keep the architecture ready for later hardware acceleration rather than shipping untested hardware transcoding initially.
- Use SQLite locally for the main v1 database.
- Support remote access in v1 when the administrator publishes Thelxinoe through their own reverse proxy and TLS setup. Passkeys are a later authentication enhancement, not a prerequisite for remote access.
- Keep a dedicated constrained Docker controller between the remotely reachable backend and the Docker daemon; do not mount the raw Docker socket into the main backend.
- Include automatic intro/outro/credit segment support in v1, alongside embedded chapter navigation, collections/franchises, Continue Watching, and episodic Next Up.
- Keep backend, Docker controller, web frontend, and Tauri client in one monorepo with one shared product version in v1. A single release tag produces all artifacts; deployment/update mechanics may differ by artifact without giving them separate product versions.
- Use one canonical container-side `/media/...` namespace for Thelxinoe-created media-stack services so Radarr, Sonarr, Lidarr, NZBGet, and Thelxinoe see the same paths. Prefer placing downloads and final media on the same filesystem so hardlinks work. External services must use the same host directory mounted at `/media`, with the same canonical library paths. Individual child mounts and path translation are unsupported.
- Let regular users request missing media while administrators approve or reject requests and can add media directly. Shared-library mutations such as delete and unmonitor are admin-only in v1.
- Back up Thelxinoe state and managed-service appdata, but not the actual media library. V1 includes manual backup/restore plus mandatory pre-update snapshots.
- Run Google/YouTube and other suitable OAuth flows through the server using the configured public base URL. Store user platform tokens only on the backend.
- Keep only `admin` and `user` roles in the v1 UI, while implementing authorization internally as named capabilities so more granular roles can be added later.
- Target Jellyfin media-client compatibility for Movies, Shows, Music and online provider libraries rather than full Jellyfin-server compatibility. Wholphin is a required tested client, but the compatibility layer should support normal Jellyfin media clients that use the covered playback/browsing surface. Jellyfin compatibility follows the authentication transports required by the tested clients instead of forcing first-party playback-grant semantics onto them.
- Connect the backend to the privileged Docker controller over a private Unix-domain socket; do not expose the controller over TCP in v1.
- Let the Docker controller perform safe self-handoff when updating the Thelxinoe server release instead of adding a third privileged updater service. Persist the authoritative first-party deployment descriptor outside the server database so recovery can recreate the current server/controller images and specs without trusting stale bootstrap Compose values. Controller handoff uses a durable generation/lease so only one controller generation can mutate Docker.
- Encrypt stored credentials and tokens using a server master key kept separately from SQLite and include the necessary encrypted secret material in restorable backups.
- Combine local-library results with Radarr/Sonarr/Lidarr acquisition results inside the relevant domain searches, with clear availability/request/download states.
- Keep advanced managed-service UIs external: administrators can open Sonarr, Radarr, Lidarr, Bazarr, Prowlarr, or NZBGet directly rather than embedding them inside Thelxinoe.
- Support admin-configurable automatic cleanup of watched media after a grace period. Movies are deleted individually; TV media is deleted by season; Music is never subject to watched-content retention. The admin selects the users whose watched state is sufficient to trigger Movie/Show retention. Managed content remains represented in Radarr/Sonarr/Lidarr, is unmonitored and deleted through its manager, and receives an import-list exclusion where needed so list automation cannot immediately add it again. YouTube downloads use a separate watchlist/pin/retained-interest cleanup rule.
- Preserve the main YouTwitch user-facing online-media features in v1 while moving their state, credentials, playback tools, and synchronization ownership to the server architecture.
- Support Jellyfin-compatible LAN discovery and Quick Connect as part of the Movies/Shows/Music media-client compatibility layer.
- Support Windows x64 for the Tauri client and Linux x86-64 for the server in v1; keep other desktop/server architectures future-friendly but unclaimed until tested.
- Allow administrators to inspect both aggregate server viewing statistics and nominative per-user playback history/statistics.
- Keep v1 English-only.
- Build the monorepo around Rust for backend/shared services, Svelte 5 + TypeScript + Tailwind for the shared frontend, and Tauri 2 for desktop.
- Bootstrap the server with a small Docker Compose deployment containing the main server and constrained Docker controller. The server image serves the web frontend.
- Treat the shipped Compose definition as bootstrap and recovery input, not as a second long-term desired-state authority. Any recreate/recovery path must consume the controller-owned deployment descriptor containing the currently accepted immutable first-party image digests and container configuration.
- Configure Thelxinoe's own persistent host paths in Compose/.env before startup; the application only sees stable container paths such as `/var/lib/thelxinoe` and `/media`.
- Use curated managed-service images in v1, record immutable image digests for installed versions, and keep arbitrary image definitions out of the UI.
- Only manage stable upstream releases. Thelxinoe will never opt managed services into nightly, develop, beta, preview, or other unstable release channels.
- Keep yt-dlp and Streamlink independently updateable on the backend while slower-moving native media tooling ships with the server image.
- Reuse the YouTwitch desktop tool-management work as an MPV-only Tauri tool manager; yt-dlp, Streamlink, and FFmpeg no longer need desktop management.
- Automatically wire Thelxinoe-created Radarr/Sonarr/Lidarr/Bazarr/Prowlarr/NZBGet instances together where their APIs allow it. Every managed service remains optional.
- For Thelxinoe-created services, keep managed service appdata below the configured Thelxinoe state root. Adopted containers may retain existing appdata locations.
- Keep v1 English-only. Do not add localization/i18n complexity until there is a concrete second-language requirement.
- Bootstrap the server with Docker Compose using two Thelxinoe containers: the main server (which also serves the web frontend) and the constrained Docker controller.
- Configure host-side persistent paths in Compose/`.env` before first launch. The first-run wizard configures logical application settings inside those mounted paths rather than choosing where the running containers are mounted on the host.
- Keep the implementation stack Rust + Tokio/Axum-style HTTP services, Svelte 5/TypeScript/Tailwind, and Tauri 2 in one monorepo/Cargo workspace.
- Use curated managed-service images in v1, record immutable image digests for installed versions, and do not expose arbitrary image/container definitions.
- Keep yt-dlp and Streamlink independently server-managed so urgent extractor/plugin updates do not require a full Thelxinoe image release; slower-moving FFmpeg/FFprobe and analysis dependencies ship in the server image.
- Reduce the desktop tool manager to MPV-related concerns; yt-dlp, Streamlink, and FFmpeg no longer belong to the Tauri client.
- Automatically wire newly created managed services together where their APIs allow it, while leaving genuine user choices such as indexers, Usenet credentials, quality profiles, and languages to the administrator.
- Keep every *arr/download service optional. Thelxinoe still functions as a media server and online-media client without them.
- Let administrators perform manual Radarr/Sonarr/Lidarr release searches and grabs using the manager's own scoring/rejection information.
- Make the responsive web client installable as a PWA in v1 without adding offline media playback.
- Keep cross-device playback control and synchronized watch-party playback out of first-party v1 scope.
- Write v1 backups to local or Compose-mounted filesystem destinations; off-host replication remains external to Thelxinoe.
- Keep the v1 HTTP API clean and versioned for first-party clients without yet promising a supported external automation API.
- Preflight managed-service updates against a consistent disposable snapshot of the live appdata before replacing the live container. Reuse the same isolated validation boundary after migration and before production activation so a failed candidate can be rolled back before it can mutate live peers or media. Verify official Thelxinoe releases through a signed manifest and immutable image/artifact digests.
- Store the server credential-encryption master key outside SQLite as a restricted persistent key file. Each self-hosted installation supplies its own provider application credentials where required.
- Provide a redacted admin diagnostic bundle and a bounded `/var/cache/thelxinoe`-style work/cache area with automatic cleanup and disk-space safety checks.
- Keep Music self-hosted and filesystem-based. External music providers, audiobooks, podcasts, Photos, and Live TV/IPTV/DVR are outside the current scope.
- Make playlists visible across users while retaining an owner; other users can play and favorite them.
- Run one main backend process with a persistent internal job scheduler/queue. Use versioned HTTP plus WebSocket for first-party clients and short-lived playback grants for first-party media URLs. The Jellyfin compatibility boundary may also accept restricted revocable compatibility device tokens using the standard transports required by tested Jellyfin clients.
- Defer true background Web Push; notifications remain stored on the server and reach connected web/PWA/Tauri clients in realtime.
- Preserve watched/resume state across manager-driven file replacements by attaching user state to logical media identity. Movie editions share logical watched/Favorite/Watch Later state but keep edition-specific resume positions.
- Configure domain acquisition defaults for normal-user requests, while admins may override them. Requests require approval by default, with optional per-user auto-approval.
- Keep v1 free of project-operated telemetry/crash reporting. Encrypt portable backups containing secret-restoration material, store timestamps in UTC with server/user display timezones, retain non-sensitive history when platform accounts are disconnected, and allow admins to disable YouTube downloading globally.

Thelxinoe is closest to a full media-server model, but it should be described as using and indexing the media catalog rather than owning the files or being their sole authority. The filesystem under configured media roots determines what local media exists and can be played. Radarr, Sonarr, Lidarr, manual copies, and other tools may populate or change those files. Thelxinoe must not interfere with the external managers' normal work.

Thelxinoe may delete media. Manager ownership is explicit: a concrete file is classified as managed, confirmed unmanaged, unresolved, or ambiguous. If a file is managed by Radarr, Sonarr, or Lidarr, destructive and monitoring actions go through that manager's API. Direct filesystem deletion is allowed only for a freshly confirmed unmanaged file. A manager outage, stale reconciliation, an unsupported library path, historical manager ownership, or a conflicting match produces unresolved/ambiguous state and blocks destructive work. A failed manager API action never falls back to direct deletion. Thelxinoe does not autonomously rename, move, or reorganize library media. Manager bindings come from exact API/file reconciliation and verified shared media mounts rather than fuzzy title matching.

Administrators define one or more named Movies, Shows, and Music roots. Thelxinoe scans them recursively, uses filesystem notifications where available, periodically reconciles the filesystem as a reliable fallback, and provides a manual scan action. Thelxinoe keeps enriched metadata, artwork, technical media data, and manual match corrections in its own storage rather than writing sidecars into the media roots.

The Docker backend is the single authority for playback sessions, watched state, progress, and history. For local media it prefers direct play, then remuxing, then FFmpeg transcoding when needed. Tauri uses MPV for playback, with its normal video window for video and headless playback controlled by Thelxinoe for music. yt-dlp and Streamlink run on the backend so clients do not need platform-specific playback tools. Wholphin compatibility exposes Movies, Shows and Music alongside separate YouTube, Twitch and Kick video libraries and private YouTube watchlists.

V1 includes automatic media-segment support for TV playback. Thelxinoe can detect recurring segments locally from the actual episode files and supplement them with external timestamps such as TheIntroDB. Intro, recap, credits, and preview segments are stored independently of the detection source and exposed to first-party clients and Wholphin. Each user chooses Auto, Ask, or Ignore per segment type, defaulting to Ask.

YouTube downloads are stored once on the server and shared physically across users while pinning, watchlists, watched state, and related ownership state remain per user. Google OAuth is used for YouTube Data API account features and is not an extractor credential. V1 yt-dlp playback/download is limited to content that the server can extract without viewer cookies or other per-user entitlement credentials; account-required extraction is deferred until a separate extractor-credential model exists. V1 clients remain server-dependent and do not provide offline playback. Browser direct play uses authenticated HTTP range requests; server remux/transcode uses HLS. There is no artificial household playback limit, and live Twitch/Kick sessions are not fan-out shared in v1.

V1 also includes administrator-controlled automatic retention. Movies are considered individually and TV is considered by complete season; the administrator chooses which users' watched state is sufficient to make each domain eligible for cleanup, rather than requiring every user to have watched it. Music is never deleted from watched-state retention. Complete-season safety prevents currently airing/uncertain seasons from being removed, and Season 0/Specials are excluded by default. Eligible items wait through a configurable grace period, and explicit Keep protection always overrides cleanup. Every destructive job stores the logical target plus the expected current concrete-file generation and revalidates file identity, manager ownership, protection state, and active Thelxinoe work immediately before execution. Automatic direct deletion of confirmed-unmanaged media is opt-in per library root; on roots with uncoordinated external writers, automatic retention stops at a pending action requiring an administrator. YouTube downloads use a separate rule: a shared download becomes eligible only when no user still keeps it in a watchlist or through another explicit keep/pin reference, then waits through its own grace period before the physical file is removed.

TV cleanup removes and unmonitors only the eligible completed season, never future seasons of the same series. Shared Movie/Show Keep protection is controlled by administrators. Normal users still have per-user Favorites for Movies/Shows/Music and a per-user Watch Later list for Movies/Shows, but those user lists do not override Movie/TV retention. YouTube remains different: any user's watchlist or explicit keep/pin protects the shared downloaded file.

Jellyfin media clients should connect directly to Thelxinoe rather than through Jellyfin for Movies, Shows, and Music. Thelxinoe implements the media-client-compatible subset it needs for browsing, metadata, playback, progress, watched state, playlists/favorites, and media segments. Wholphin is a required compatibility target, but full Jellyfin server/admin/plugin compatibility is not a goal. The compatibility contract includes the concrete authentication, image, subtitle, direct-play, remux/transcode, and progress request patterns exercised by the tested clients, including standard token transport where those clients construct URLs themselves.

For reverse-proxy deployments, Thelxinoe must understand its configured public base URL, trust forwarded scheme/host/client-address headers only from explicitly trusted proxies, and keep browser APIs same-origin where possible. CORS is not a substitute for proxy/public-URL configuration and should only be enabled for explicit origins that genuinely need cross-origin API access.

The first-run UI does not choose host-side Docker paths. Those mounts already exist before the server starts. The mandatory wizard is therefore limited to application-level setup such as the first administrator, timezone, logical library roots exposed inside the container, optional public URL/proxy settings, and optional media-stack installation. V1 is English-only.

Host filesystem layout is deployment configuration rather than first-run application configuration. The Compose/`.env` layer provides Thelxinoe's persistent state root and the host media/download roots that are visible inside the containers. The backend uses fixed internal paths such as `/var/lib/thelxinoe` and `/media/...`. The setup UI selects and classifies directories that are already mounted into the server; it does not attempt to invent new host bind mounts from inside the container. Every integrated media service must bind the same host media root at `/media`; individual library overrides are not supported.

There is no built-in YouTwitch migration requirement. One manual database migration may be performed separately during the transition, but migration code is not part of Thelxinoe.

## Existing code available for reuse

The sibling `../youtwitch` project has useful code in both halves of the application.

Frontend areas include shared Svelte UI components, YouTube feed and watchlist logic, Twitch and Kick views, settings, statistics, and presentation helpers.

Rust areas include platform APIs, OAuth, SQLite repositories and migrations, playback management, downloads, statistics, and managed media tools.

The current frontend talks directly to Tauri through `src/lib/api.ts`. Thelxinoe uses a server-facing transport; Tauri-specific native functions remain separate from server functions.

## Maintaining these contracts

Changes to recovery guarantees, destructive-action safety, identity mapping, authentication compatibility or deployment ownership require an explicit update to the relevant design document. See [the documentation index](README.md) and [implementation evidence](STATUS.md).
