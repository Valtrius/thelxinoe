# Online providers acceptance

These are the accepted v1 delivery criteria. Current results and supported limits are in [implementation status](../STATUS.md) and the [release checklist](../RELEASE_CHECKLIST.md).

## Phase 9. YouTube server migration

Implement YouTube account features and playback on the server.

Deliver:

- per-install Google application credentials;
- per-user YouTube Data API OAuth on the server, explicitly separate from media-extractor authentication;
- OAuth callback/public-base-URL integration tests through the supported HTTPS reverse-proxy model;
- subscriptions/feed sync;
- Shorts filtering;
- live/replay handling;
- shared quota budget and fair scheduling;
- per-user watchlists;
- server-side yt-dlp tool management;
- public-extraction-only yt-dlp playback/download in v1, with no Google OAuth token, browser cookie, or other per-user entitlement credential passed to the extractor;
- explicit `extractor authentication required` state for metadata-visible media that cannot be played/downloaded under that rule;
- shared physical downloads with per-user state only for media whose extraction did not depend on one user's private entitlement;
- download retention based on watchlist/keep/pin interest;
- global admin toggle for YouTube downloads;
- disconnect versus delete-platform-data behavior.

Exit condition:

- web and desktop clients support YouTube feeds, filtering, watchlists, downloads and playback, with credentials and extraction tools managed by the server.

## Phase 10. Twitch and Kick server migration

Deliver:

- per-user Twitch connection;
- followed-live synchronization;
- server-side Streamlink playback;
- per-user tracked Kick channels;
- optional Kick metadata credentials as required by current APIs;
- live playback/session statistics;
- disconnect/data-deletion semantics;
- retained rate-limit handling.

Exit condition:

- YouTube, Twitch, and Kick all use the same server-owned auth/playback/session model.
