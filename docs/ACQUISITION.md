# Acquisition development milestone

Administrators can connect local Docker Radarr, Sonarr and Lidarr containers in Settings, choose existing root folders and profiles, and enable automatic approval for individual users. Requests search the manager and local catalog. Ordinary requests await approval; administrators and users with automatic approval enqueue durable add/search work. A search interrupted after submission requires review instead of an automatic repeat.

Manager API keys are encrypted. The private Docker controller returns limited network/mount evidence and never returns container environment variables or commands. The server resolves the internal address from a shared Docker network and requires one writable `/media` bind backed by exactly the same host directory in both containers. Child mounts are rejected. Radarr, Sonarr and Lidarr must use `/media/movies`, `/media/tv` and `/media/music` respectively. A changed mount source blocks use until the administrator reconnects the service. Docker Desktop Windows host paths and Linux host paths are supported as mount evidence.

Request status can inspect availability and downloads. Administrators can change monitoring, inspect manager release scores/rejections, and explicitly grab an approved release. Request again returns a previously submitted request to approval or queues it according to the user's policy.

## File operations

Ownership reconciliation stores manager entity/file IDs, exact episode/track IDs and mapped paths for each concrete file generation. An unavailable manager retains its historical evidence and makes ownership unresolved. Multiple claims are ambiguous. Both states block destructive operations.

The media detail panel offers Keep protection and reviewed monitor/unmonitor/delete operations. Preparation captures the complete file set, generations, fingerprints and manager claims. Execution holds a server media-operation lease shared with scanning and playback creation, rechecks active playback and Keep, hashes the physical files, refreshes ownership, and checks manager commands/download activity. A file shared with logical media outside the selection blocks the command. Movie/album monitoring cannot silently widen a partial selection to other files.

Managed deletion uses the owning manager's file API and unmonitors the exact unit. Sonarr actions use its recorded episode IDs, never the episode number in a filename. A failed manager call never falls back to filesystem deletion. Explicit administrator deletion of confirmed-unmanaged files uses the filesystem. Interrupted or partially executed commands remain uncertain and are not automatically replayed. Completion requires the files to be absent and catalog reconciliation to finish.

## Isolated validation

The fixture deployment uses ports 23443 (Thelxinoe HTTPS), 27878 (Radarr), 28989 (Sonarr) and 28686 (Lidarr). State and generated media are confined to `.local/acquisition`. Never point these scripts at a real media library.

```powershell
node scripts/acquisition-fixtures.mjs
docker compose -f compose.acquisition.test.yaml up -d --wait
node scripts/test-acquisition.mjs
node scripts/import-acquisition-fixtures.mjs
node scripts/test-manager-files.mjs
node scripts/test-manager-tv-music.mjs
```

Real adapter validation used Radarr 6.4.4.10685, Sonarr 4.0.20.3014 and Lidarr 3.1.0.4875, pinned by image digest. The browser test registers services, selects defaults and submits approved requests through HTTPS. Fresh-artist Lidarr acquisition was also exercised with Blue Train. Generated video/audio files are manually imported through the real manager APIs, then bound and deleted through Thelxinoe. The Sonarr fixture deliberately maps a filename containing S01E01 to Sonarr episode 2: deletion unmonitored episode 2 and preserved episode 1.

Sanitized results are `.local/acquisition-result.json`, `.local/manager-files-result.json` and `.local/manager-tv-music-result.json`. Rust tests cover permission checks, encrypted credential redaction, duplicate requests, interrupted search, changed mounts, sticky ownership, Keep, content replacement with unchanged size/timestamp, and replay prevention.

## Supporting services and download validation

Settings also registers local Bazarr, Prowlarr and NZBGet containers. Their encrypted credentials never reach ordinary users. Prowlarr health/indexer tests and enable/disable controls, Bazarr wanted subtitles and acquisition, and NZBGet queue/history, pause/resume/remove and speed controls are available with native UI links. Completed downloads retained in NZBGet duplicate history remain visible after manager import cleanup.

The private Newznab and NNTP fixtures publish generated black video, silent audio and a generated subtitle. Thelxinoe release search/grab passed through Prowlarr 2.6.5.5623 to NZBGet 26.3, then automatic Radarr movie, Sonarr episode and complete Lidarr album imports. Sonarr release search explicitly selects a manager season; a regression test prevents falling back to unrelated RSS results. Bazarr 1.6.1 extracted the embedded English subtitle and removed the movie from its wanted list. No commercial subtitle or indexer account is required for these tests.

After the preceding manager tests, run:

```powershell
node scripts/generate-acquisition-news.mjs
node scripts/configure-acquisition-downloads.mjs
node scripts/test-support-services.mjs
node scripts/configure-bazarr-fixture.mjs
# Run the download test once to acquire files before the subtitle fixture exists.
node scripts/test-acquisition-downloads.mjs --downloads-only
node scripts/embed-bazarr-fixture.mjs
node scripts/test-acquisition-downloads.mjs
```

The test-only native ports are 26767 (Bazarr), 29696 (Prowlarr) and 26789 (NZBGet). The indexer and NNTP server have no host ports. Sanitized evidence is in `.local/support-services-result.json` and `.local/acquisition-downloads-result.json`. The support browser script exercises registration, health, queue and rate controls. Linux/Windows manager regression tests cover admin-only access, secret redaction, forbidden RPC commands and stale queue selections.

## Remaining scope

Acquisition is available in the Requests section and through Search and request in Movies, Shows and Music. Automatic retention, import-list exclusion management, managed Docker installation/update ownership, and recovery/backup are later roadmap work. The controller currently exposes read-only inspection only.

The operation lease coordinates this server's work. Arbitrary programs writing directly into media roots remain outside its control, as described in the architecture. Manager activity checks are conservative: any active command or download queue blocks destructive work for that manager.

## Requests and acquisition

Domain search combines local results with acquisition results from Radarr, Sonarr, or Lidarr.

Admins configure default acquisition settings per domain, including root folder, quality profile, monitoring rules, and language-related choices. Normal users request media using those defaults. Admins can override defaults on manual adds.

Requests need admin approval by default. Admins can mark individual users as auto-approved.

Admins can also run manual release searches and explicit grabs. Thelxinoe displays the manager's own release scores and rejection reasons rather than reproducing its ranking logic.

## Manager binding and destructive actions

Thelxinoe reconciles local file paths with Radarr, Sonarr, and Lidarr API file records. Managed and external containers share the same canonical `/media` bind and library paths. File paths are used directly; there are no path mappings.

A concrete file has one of four ownership states:

- `managed`: a current reconciliation proves one owning manager instance and records its stable manager entity/file IDs;
- `unmanaged`: a current successful reconciliation against every enabled relevant manager proves that none owns the file;
- `unresolved`: ownership cannot currently be proven because a relevant manager is unavailable, a file path is outside the required library root, or previous ownership evidence cannot yet be refreshed;
- `ambiguous`: more than one manager currently claims the file.

Historical ownership is sticky evidence. A previously managed file does not become unmanaged merely because its manager is offline, its mount mapping breaks, or a reconciliation record expires. It becomes unresolved until ownership can be proved again.

If one file matches multiple managers, Thelxinoe marks the binding ambiguous and blocks manager-routed destructive actions until the conflict is resolved.

Before a destructive or monitoring action, Thelxinoe refreshes the ownership state when the last successful reconciliation is no longer current enough for that operation. Managed deletion and monitoring changes go through the owning manager API. Direct filesystem deletion is allowed only for confirmed-unmanaged files. Unresolved and ambiguous ownership blocks the action. A manager timeout, rejection, or API failure never causes a fallback to direct filesystem deletion.

TV bindings retain the exact manager episode/file identities associated with each logical episode. Season-level actions translate the Thelxinoe logical season into the exact set of manager-owned episodes/files. Matching numeric season and episode coordinates alone is insufficient. If provider/order mapping is not one-to-one or cannot be proven, automated season-level monitoring/deletion is blocked until the mapping is corrected.

Thelxinoe does not autonomously rename, move, or reorganize media.
