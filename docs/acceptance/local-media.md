# Local media and clients acceptance

These are the accepted v1 delivery criteria. Current results and supported limits are in [implementation status](../STATUS.md) and the [release checklist](../RELEASE_CHECKLIST.md).

## Phase 3. Local library indexing

Build filesystem-first Movies, Shows, and Music discovery without playback yet.

Deliver:

- named typed library roots;
- recursive scan;
- filesystem watcher integration;
- periodic reconciliation scan;
- persistent file index;
- opaque logical Movie, Show, Season, Episode, Artist, Album, and Track identities that do not derive identity from path or displayed provider numbering;
- embedded music tag parsing, including embedded MusicBrainz identifiers when present, so Artist/Album/Track identity does not need to be rewritten when network metadata arrives later;
- technical media probing with FFprobe;
- specials, multi-episode files, multi-disc albums, and basic editions;
- scan status/progress events;
- manual scan action;
- basic Movies/Shows/Music browsing UI.

Exit condition:

- manually copied media appears after scan/watch events;
- removing/replacing files updates the catalog without resetting logical identity where it can be matched safely.

## Phase 4. Metadata and artwork

Turn the raw file index into a useful media catalog.

Deliver:

- TMDB integration for Movies/Shows;
- MusicBrainz and Cover Art Archive integration;
- provider-ID storage;
- provider-specific TV series/episode identifiers and ordering coordinates stored alongside the internal identity;
- explicit mapping between logical TV episodes and provider/order identities, including non-one-to-one and unresolved cases;
- matching pipeline;
- artwork cache;
- metadata refresh jobs;
- manual match/rematch;
- manual field overrides stored separately from provider data;
- TMDB collections/franchises;
- trailer metadata and discovered local trailer playback metadata.

Exit condition:

- the three local domains show stable metadata/artwork;
- refresh does not overwrite manual corrections.

## Phase 5. Local playback and progress

Build the playback session model once and use it for every later client.

Deliver:

- playback-session schema/state machine;
- direct-file HTTP range streaming;
- HLS remux/transcode pipeline;
- software FFmpeg transcode;
- playback authorization that resolves a user/session principal independently of whether a request later arrives with a first-party grant or a compatibility credential;
- client capability model;
- Auto, Original, and bitrate/quality preferences;
- audio/subtitle track discovery and selection;
- per-user audio/subtitle preferences;
- playback progress and watched-state reporting;
- 90% watched inference;
- logical-media resume handling across file replacement;
- edition-specific resume handling;
- concurrent session handling;
- bounded transcode/cache cleanup;
- browser music sequencing that supports gapless transition semantics for supported formats/cases;
- ReplayGain tag ingestion/application for browser playback;
- HTTP range and HLS playback tests through a representative HTTPS reverse proxy.

Exit condition:

- supported browsers can play representative direct-play, remux, and transcode cases on LAN and through the supported reverse-proxy path;
- representative adjacent music tracks play gaplessly where the chosen browser/container/codec path supports gapless delivery, with ReplayGain applied according to the stored tags;
- progress survives restart and file replacement;
- two clients can play independently.

## Phase 6. Tauri MPV playback

Build native playback on the server playback contract.

Deliver:

- MPV-only tool manager;
- managed MPV and use-my-installation modes;
- MPV config/plugin support retained where still useful;
- native video playback launch;
- headless MPV music playback;
- MPV gapless queue transitions and ReplayGain behavior aligned with the server music model;
- MPV IPC to server playback-session reporting;
- desktop notifications;
- one-server connection/change-server settings;
- desktop update hook placeholder tied to the shared product release.

Exit condition:

- Tauri plays Movies/Shows and Music through MPV while the server remains the progress authority.

## Phase 7. Multi-user media state

Complete the user-facing state model before external services depend on it.

Deliver:

- Favorites for Movies/Shows/Music;
- Watch Later for Movies/Shows;
- owned server-visible playlists;
- playlist favoriting by other users;
- persistent per-client music queues;
- Continue Watching;
- Next Up;
- playback history/statistics;
- admin aggregate statistics;
- admin nominative per-user history/statistics;
- per-user display timezone;
- audit log for admin/destructive actions.

Exit condition:

- two users can use the same library with independent progress/state and shared playlist visibility.

## Phase 8. Jellyfin media-client compatibility

Add the compatibility layer against the existing catalog and playback model.

Deliver:

- Jellyfin server-info endpoints required by clients;
- initial protocol spike against the current Wholphin build and one additional Jellyfin media client before freezing adapter auth/URL assumptions, covering login/device identity, image URLs, subtitle URLs, direct play, remux/transcode/HLS, seeking, and progress reporting;
- revocable compatibility-device authentication mapping using the standard credential transports exercised by the tested clients;
- libraries/views;
- item browsing/filter/search;
- images;
- playback-info/device-profile translation;
- server-generated media URLs backed by short-lived Thelxinoe playback grants where the client can consume them, while accepting compatibility-device token transport where an unmodified client constructs standard Jellyfin URLs itself;
- progress/watched translation;
- Favorites/playlists needed by target clients;
- UDP LAN discovery;
- Quick Connect;
- redaction tests proving `ApiKey`, playback grants, and equivalent query credentials do not appear in normal access logs or diagnostic bundles;
- compatibility tests against Wholphin;
- at least one additional Jellyfin media client in the test matrix.

Exit condition:

- Wholphin can discover/connect, browse Movies/Shows/Music, play media, resume, and report watched state without Jellyfin running.
