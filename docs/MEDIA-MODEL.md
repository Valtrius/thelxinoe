# Catalog, metadata and playback model

## Local media catalog

Administrators configure one or more named library roots. Each root has an explicit type: Movies, Shows, or Music.

The server scans recursively. Filesystem events provide fast updates where available. Periodic reconciliation scans remain the reliable source for missed events. Administrators can trigger a manual scan.

File presence determines whether media is playable. Thelxinoe maintains its own logical catalog and metadata index.

Important identity rules:

- Logical media rows use opaque internal IDs. Paths, titles, provider IDs, season/episode numbers, disc/track numbers, and manager IDs are evidence attached to an internal identity, not the identity itself.

- Movie and Episode progress survives file replacement or quality upgrades.
- Concrete files remain separate records for technical media information and playback sources.
- Movie editions share watched, Favorite, and Watch Later state but keep edition-specific resume positions.
- TV specials, multi-episode files, multi-disc albums, and basic movie editions are modeled in v1.
- One logical episode can store identifiers and ordering coordinates from multiple providers/managers. A concrete file can represent one or more logical episodes without forcing all providers to use the same season/episode numbering.
- Provider display ordering and manager action mapping are separate. The UI may primarily display TMDB ordering while a Sonarr binding retains the Sonarr/TVDB episode identity needed for manager operations.

## Metadata

Movies and Shows use TMDB as the main external metadata and display source. TMDB numbering is not assumed to be interchangeable with another provider's or a media manager's episode numbering.

Music uses embedded tags for local facts, MusicBrainz for identity and metadata, and Cover Art Archive for artwork.

Existing sidecars and manager IDs can help matching but do not own Thelxinoe's catalog. Provider-specific series/season/episode IDs and ordering data are retained where available so manager bindings can refer to the exact external entity instead of inferring identity from a displayed `SxxExx` coordinate.

Provider-managed fields refresh periodically and on demand. Manual admin corrections live separately and always win until the admin clears them.

## Playback model

The backend owns playback session state for every client type.

Local media delivery order is direct play, then remux, then software FFmpeg transcode. Hardware transcoding is not in v1. The transcoding interface must allow a hardware implementation later without changing client APIs.

Browser direct play uses authenticated HTTP range requests. Remuxed and transcoded browser playback uses HLS.

The Windows Tauri client uses MPV for playback. Video opens the normal MPV window. Music uses headless MPV controlled by the Thelxinoe frontend.

The Tauri tool manager keeps MPV management and MPV configuration/plugin support. yt-dlp, Streamlink, and FFmpeg do not run as desktop-managed tools.

Clients advertise playback capabilities. The server supports Auto, Original, and explicit quality/bitrate limits.

Audio and subtitle preferences are per user, with per-session manual overrides. Bazarr handles subtitle acquisition for managed Movies/Shows. Thelxinoe also discovers embedded and external subtitle tracks itself.

## Music

Music is a self-hosted local library. External subscription music services are not part of the current product scope.

V1 music playback includes artists, albums, tracks, gapless playback, ReplayGain, favorites, playlists, and persistent per-client queues.

Playlists have an owner but are visible to all users. Other users can play and favorite them. Collaborative editing is not required in v1.

Audiobooks and podcasts are separate future domains, not partial Music modes.
