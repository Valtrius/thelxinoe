# Jellyfin client compatibility

The adapter implements roadmap phase 8's core client workflows. It uses Thelxinoe's catalog, authentication sessions, playback grants, progress, and history. It does not run a Jellyfin server or expose Jellyfin administration APIs. Broader compatibility and the limits below remain part of release hardening.

## Protocol findings

- Wholphin 1.0.8 uses Jellyfin Kotlin SDK 1.7.1. Discovery requires `ProductName: "Jellyfin Server"`. The adapter keeps the actual product and release in `ServerName` and `ThelxinoeVersion`; `Version: "10.10.7"` identifies the compatibility contract. Jellyfin Android TV 0.19.10 currently warns that its next release will require 10.11; that newer contract still needs validation.
- Kotlin model constructors require more fields than the OpenAPI `required` lists suggest. Authentication includes the required user policy, configuration, and session fields. Playback sources and streams include the required booleans and protocol enums.
- Clients repeat list query parameters, including fields and image types. The adapter merges documented lists and rejects conflicting scalar parameters, including credentials and user identity.
- API requests use a revocable compatibility device token. Header and query credential transports are restricted to compatibility routes. They cannot authenticate first-party APIs, and first-party tokens cannot authenticate the compatibility API.
- Wholphin's gallery sends the device authorization header, but its video player uses an unauthenticated media data source. It constructs `/Videos/{id}/stream` and preserves the source `ETag` as `tag`. Playback-info therefore places a short-lived, session-bound playback grant in that field. The direct URL returned by the adapter also uses the existing granted playback endpoint. Stopping or revoking the parent device invalidates these URLs.
- Jellyfin Android TV also preserves `ETag` but omits `PlaySessionId` from its direct URL. The adapter resolves the grant to its exact playback, media and source; conflicting explicit identifiers are rejected.
- HLS uses the compatibility transcoding path even when the server copies codecs. This tells clients that audio selection is controlled by the server. Advertising progressive direct-stream support for HLS causes Jellyfin Android TV to repeatedly restart playback while trying to select an audio track locally.
- Wholphin music requests authenticated `/Audio/{id}/universal` URLs without playback-info and reports progress without a play-session ID. The adapter maintains an association scoped to the authenticated device and track. This path currently supports the advertised original audio formats; conversion is available through playback-info.
- Artwork tags carry short-lived artwork grants so the client's player artwork request can authenticate too. Authenticated gallery requests remain supported.
- Wholphin requests cinema intros before starting playback. The adapter returns an empty intro list because no cinema intro source is configured.
- Quick Connect stores hashes of its secret and six-digit code, expires after five minutes, and requires a signed-in user to inspect and approve the named device. Approval is tied to that user's session with a separate, single-use confirmation grant. The client exchanges the approved secret once. Polling must retain the same display code; Jellyfin Android TV redraws it from every poll result.
- Compatibility debug logs contain only an allowlisted route shape, method, and status. They omit arbitrary path components, all query values, headers, and bodies.

## Local validation

The local test server uses Docker project `thelxinoe-compat`, HTTP port 18787, HTTPS port 21443, and a dedicated state volume. The Android TV API 34 emulator is `emulator-5580`; its host address is `10.0.2.2:18787`.

| Check                                  | Wholphin 1.0.8                                                     | Jellyfin Android TV 0.19.10                             |
| -------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------- |
| Manual server connection               | Passed                                                             | Passed                                                  |
| Password login                         | Passed                                                             | Passed                                                  |
| Quick Connect through Thelxinoe web UI | Passed                                                             | Passed                                                  |
| Library navigation and movie list      | Passed                                                             | Passed                                                  |
| Real cached TMDB artwork               | Passed                                                             | Passed                                                  |
| Direct H.264/AAC playback              | Passed                                                             | Passed                                                  |
| Progress, completion, history          | Passed: 18-second clip, 17.901 seconds counted; episode completion | Passed: video progress and both music tracks completed  |
| Subtitle display and selection         | Passed with timed SRT cues; limitation below                       | Passed after seek                                       |
| Remux, transcode, seek and resume      | Passed: 90-second fixtures, forward/back seek and saved resume     | Passed, including resume saved by Wholphin              |
| Music playback                         | Passed: two-track FLAC queue and progress                          | Passed: two FLAC tracks, 3 seconds counted each         |
| Shared playlists and favorites         | Passed: ordered duplicate tracks and movie favorite persisted      | Passed: playlist detail and playlist favorite persisted |
| Shows                                  | Passed: specials, seasons, episode playback/completion             | Not exercised separately                                |
| UDP discovery                          | Host protocol passed; emulator broadcast not verified              | Same                                                    |

`scripts/test-jellyfin.mjs` validates authentication, browsing, search, token transport separation, granted direct byte ranges, progress and watched translation, stopped-stream rejection, and absence of token/grant values from Docker logs. It uses synthetic playback fixtures. `scripts/prepare-tv-metadata.mjs` explicitly uses the private local provider files to attach real TMDB artwork to that synthetic fixture. `scripts/test-tv-quick-connect.mjs` approves the code currently displayed in the emulator through the real web interface.

Rust integration tests cover device revocation, cross-user rejection, credential conflicts, and the Quick Connect approval and single-use boundaries. All previous server integration tests still pass.

Converted compatibility playback exposes a complete HLS VOD timeline. Segments are generated on demand, so a client can seek forward before earlier segments have been prepared, or seek backward after eviction. Remuxed video uses indexed keyframe intervals; transcoding uses six-second intervals. Work shares the four conversion slots with first-party playback, is canceled when playback stops, and has duration, time, free-space, and cache-size limits.

The Docker protocol test requests the final segment before the first for both remux and transcode, decodes the results, and verifies stopped sessions lose access. It also checks profile-constrained resolution/frame rate, HDR-to-SDR conversion, SubRip/WebVTT delivery, device-isolated universal audio, and encoding cancellation. Rust tests compare every decoded video frame across all segments against timestamped MKV, AVI and shifted-timestamp fixtures, and check audio-only seeking and regeneration after eviction.

Wholphin evidence includes `.local/tv-sub-multiple.png` (remux seek to 47.667 seconds with the 40–50 second subtitle), `.local/wholphin-remux-subtitle-back.png` (backward seek), `.local/wholphin-transcode-subtitle.png`, `.local/wholphin-transcode-resume.png` (resume at 67 seconds), and `.local/wholphin-music.png`. Jellyfin Android TV direct playback is captured in `.local/jellyfin-tv-direct.png`.

Jellyfin Android TV remux/subtitle and transcode evidence is in `.local/jellyfin-tv-remux-subtitle.png` and `.local/jellyfin-tv-transcode.png`; music and shared playlists are captured in `.local/jellyfin-tv-music.png` and `.local/jellyfin-tv-playlist.png`. `.local/tv-playback-history.json` records progress from the actual clients. The scanner's recent job list remained unchanged while clients browsed and played, confirming reads do not cause repeated scans.

Known limits: Wholphin can lose an already-active, single long subtitle cue after a seek, with either external WebVTT or SubRip; ordinary timed cues display after forward/back seeks and resume. Forced subtitle flags and forced-only selection are supported, but that selection mode has not been exercised in the TV UI. Universal audio conversion for clients that bypass playback-info is not yet supported. UDP discovery replies are tested on the host; the Android emulator's NAT does not provide a normal LAN broadcast path. Music playlists are edited in the first-party application; compatibility clients can browse, play and favorite them. Private YouTube watchlists also support creation and membership editing through the compatibility API. Person/genre entity indexes, theme media, administration and remote control are not exposed.

Normal access-log credential redaction and diagnostic export are covered. The diagnostics fixture authenticates a compatibility device, exercises query-token transport, stores a credential-bearing error/audit target and verifies the exported allowlist excludes those values and all authentication tokens. A compatibility token cannot authorize the export. Normal LAN broadcast discovery remains a deployment acceptance check because the emulator has no normal LAN broadcast path. Supporting newer client versions requires repeating this matrix.

## Jellyfin media-client compatibility

Thelxinoe targets Jellyfin media-client compatibility for Movies, Shows, Music and separate online provider libraries. It does not aim to implement the whole Jellyfin server API.

The compatibility layer includes server discovery and server info, authentication/device identity, Quick Connect, libraries/views, browsing/filter/search, images and metadata, Favorites and playlists where applicable, playback info and capability negotiation, direct/remux/transcode URLs, playback progress/stopped/watched state, and Media Segments.

Wholphin is a required tested client. Full Jellyfin administration, plugin APIs, Live TV backend, task APIs, and unrelated server endpoints are outside the contract.

## Online libraries and watchlists

YouTube, Twitch and Kick appear as standard video libraries in Wholphin. YouTube includes the signed-in user's public subscription videos and retained watchlist/pinned videos. Twitch includes followed channels currently live; Kick includes tracked channels whose cached metadata reports them live. Account linking and channel management use Thelxinoe's web or desktop application. The TV reads the same synchronized catalog and does not request a provider refresh on every browse.

Online item UUIDs are stable and private to each Thelxinoe user. Artwork, playback and playlist operations recheck that ownership. Provider credentials and signed CDN media addresses stay on the server. Online playback uses profile-constrained H.264/AAC HLS. YouTube exposes a complete seekable timeline and shares resume, watched state and history with first-party clients; an available completed download is reused. Public remote VOD prepares up to eight adjacent six-second segments per connection, publishing each as it becomes ready. Seeking outside that window cancels it and opens a new bounded window. Live channels use the continuous two-second HLS pipeline and have no VOD resume position.

YouTube watchlists appear under **Playlists** as private video playlists. Wholphin's **Add to playlist** action can create a watchlist or add to an existing one; its playlist editor can remove or reorder entries. The saved Thelxinoe manual/date ordering is used until an explicit TV reorder switches that watchlist to manual ascending order. Public/shared YouTube playlists and adding non-YouTube media to these lists are rejected. Music playlists retain their existing shared visibility and compatibility editing restrictions.

The watchlist's existing user-defined auto-download and auto-remove-watched settings continue to apply. TV operations do not change either setting. Creating a new watchlist uses the normal defaults. Wholphin has no new standalone download command: adding a video to a watchlist whose auto-download setting is already enabled uses the existing server download policy, including the administrator's download permission.

On 2026-09-20, Wholphin 1.0.8 on the Android TV emulator connected to the real-provider development profile at `10.0.2.2:18888` through Quick Connect. It browsed 6,396 YouTube videos with artwork, created a private watchlist, played its video, removed an entry and reordered two entries with the TV arrows. The temporary test list was removed afterward. A sustained run advanced beyond two minutes with about fifty seconds buffered ahead and no playback error in the sampled player states. Twitch and Kick live playback decoded in the TV client. Protocol checks requested an unprepared later segment before the beginning, decoded both, and verified shared resume for public streaming and a completed YouTube download. Integration tests cover user isolation, stable IDs, membership and ordering changes, existing watchlist settings, and deletion cleanup.

Local evidence includes `.local/wholphin-youtube-library.png`, `.local/wholphin-youtube-window-playing.png`, `.local/wholphin-youtube-window-hold.json`, `.local/wholphin-twitch-playing.png`, `.local/wholphin-online-protocol.json` and `.local/wholphin-download-protocol.json`. Provider playback remains subject to public extraction availability; short live checks do not establish advertisement or long-running stream reliability. This new online surface has not been repeated in Jellyfin Android TV.
