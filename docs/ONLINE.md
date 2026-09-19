# Online providers

YouTube account linking, subscription synchronization, feed browsing, watchlists, public streaming and downloads, live playback, private history, pins and watched flags are implemented and validated with real providers. Twitch and Kick remain future phases.

## Google configuration

An administrator configures one Google OAuth **Web application** for the installation. Regular users connect their own Google/YouTube account; they do not create a Google developer project.

1. Set `THELXINOE_PUBLIC_URL` to the server's public HTTPS origin and configure its trusted reverse proxy. HTTP localhost is accepted for development.
2. In Google Cloud, enable **YouTube Data API v3**, configure the OAuth consent screen, and create a **Web application** OAuth client. Add test users while the application remains in testing.
3. Copy the exact **Authorized redirect URI** displayed in Thelxinoe Settings into Google's client configuration. Its path is `/api/v1/online/youtube/callback`.
4. Save the client ID and client secret in Settings → YouTube application. Replacing this configuration removes existing viewer credentials and requires those users to reconnect.
5. Each person opens YouTube → Connect YouTube and grants read access. The Windows application opens the configured server in the normal browser; the person signs in to Thelxinoe there before linking Google.

See Google's [web server OAuth documentation](https://developers.google.com/identity/protocols/oauth2/web-server). A client previously used by an installed desktop application needs its callback support checked; importing an ID and secret alone does not establish server compatibility.

## Credential boundaries

The application secret, viewer access/refresh tokens and short-lived PKCE verifiers are encrypted in server storage. Tokens are bound to the Thelxinoe user, and never returned to the frontend. Account linking uses a ten-minute, single-use attempt tied to its original active web session, a random browser cookie, PKCE S256 and the configured callback. The callback cookie is HttpOnly/Secure/SameSite=Lax and restricted to the callback path. Normal login cookies stay SameSite=Strict.

Refresh requests are serialized and reread credentials before exchanging them. Account generations prevent delayed OAuth, refresh or sync responses from restoring a disconnected/deleted connection. Provider rejection requires reconnection; temporary failures back off without erasing the connection. Provider error bodies, OAuth codes, tokens and authorization URLs are not logged by the server.

Disconnect removes this server's viewer credentials and pending authorization attempts. Saved videos and personal state remain. Delete YouTube data also removes the user's cached feed, subscriptions, watchlist, pins and watched flags. Application configuration remains administrator-owned. Neither action changes YouTwitch's credentials or local data.

## Synchronization and limits

The server reserves Data API quota before each request, including failed requests. Its shared daily ledger uses midnight in `America/Los_Angeles`; the default local budget is 10,000 units and the administrator can adjust it. A provider quota error blocks further Data API calls for that day. Ordinary provider failures use increasing retry delays up to one hour.

The persistent scheduler advances one bounded page per user on each turn, ordered by the least recent turn. A crash leaves a ninety-second lease and resumes the stored cursor after that lease expires. Subscription snapshots only remove old subscriptions after all pages complete. Initial subscription snapshots support up to 10,000 channels; each list request contains at most fifty items. Feed backfill is bounded to three upload pages per channel and stops after reaching ninety-day-old uploads. Completed cycles normally repeat after thirty minutes; manual requests have a five-minute cooldown.

Viewer-visible video metadata stays per user. Live, upcoming and replay states come from YouTube's video metadata. Live/upcoming videos and retained items are refreshed in later cycles. Videos shorter than three minutes remain unclassified until a public Shorts endpoint check succeeds; duration alone never classifies a regular video as a Short. At most fifty classifications run per cycle, with unknown results retried after a day. Unknown Shorts classifications remain visible when Shorts are hidden.

Watchlist additions create a durable placeholder immediately and resolve details on the same fair queue. Each user can retain up to 1,000 watchlist/pinned videos. Feed pages contain fifty items. Artwork is proxied for public videos after checking the requesting user's visibility; a scoped, expiring grant supports desktop image requests. No viewer credentials are attached to public thumbnail or Shorts requests.

Google OAuth covers the Data API only. Extraction receives no Google tokens, cookies or browser profiles.

## Public extraction and downloads

An administrator installs the official yt-dlp and Deno releases from Settings. Downloads are checked against GitHub's published SHA-256 asset digests. Each installation has separate executable paths; jobs pin the selected versions, and each executable is checked before use. Installation selects the new bundle only after both tools pass startup checks.

Play opens public media immediately in the browser or MPV player. The server extracts public CDN sources and converts them to the same bounded HLS pipeline used by local playback. Signed source URLs remain in server memory. VOD supports seeking and resume; live playback hides seeking and does not infer watched state or save a VOD resume position. Original quality and online caption selection require a completed download; immediate streaming currently converts to H.264/AAC up to 1080p.

The administrator separately enables YouTube downloads (disabled by default). Save or pin a video, then choose Download to retain it. Once ready, Play uses that completed file. Downloads share physical media while each user keeps separate resume, watched state and history records. Online media never enters Movies, Shows, Music or the Jellyfin catalog.

Extraction runs with a cleared environment, temporary home/config/cache directories, ignored yt-dlp configuration and plugins, no browser cookies, and no remote JavaScript component downloads. Deno is an explicitly selected, verified runtime. Process groups on Linux and job objects on Windows terminate descendants on cancellation. Metadata extraction has two slots, a two-minute timeout and bounded output. Downloads have one separate worker, a thirty-minute timeout, disk monitoring, and cancellation when downloads are disabled or no user retains the video.

The current download path supports completed videos up to six hours, 1080p and 2 GiB, with a 50 GiB retained-cache limit. Temporary merge files are monitored separately. Protected files are not evicted to make room. Live videos use immediate streaming and cannot be retained by this download path. Content needing sign-in, membership or age verification reports `extractor_authentication_required`; the app does not borrow Data API credentials to bypass that limitation.

A shared download becomes eligible for cleanup only after one day without any user's watchlist/pin interest or active playback. Cleanup rechecks the exact generation and protections under the database write lock and removes only its canonical cache directory. Deleting one user's YouTube data stops their online sessions and removes their progress/history; another user's interests continue protecting the physical file. The broader media-operation coordinator remains a later roadmap phase.

Real validation on the isolated HTTPS server passed Google consent using the imported YouTwitch application credentials, a complete 6,350-video initial sync, public artwork, official tool installation, public extraction, and downloaded-video decoding/seek/resume in Chromium. The rebuilt Windows application also passed MPV playback, seek and server resume for the public download. Automatic OAuth refresh after real token expiry also passed. Immediate VOD streaming, seeking and resume passed through HTTPS and MPV; live playback decoded frames in both clients. A rapid VOD-to-live transition test found and fixed a stale player-close callback. History offers a YouTube filter with private user records and capability-gated administrator statistics.
