# Watched media retention

Settings contains separate movie and TV policies. Both start disabled, with a seven-day grace period. Select one or more users whose watched status can trigger cleanup. Any selected user is sufficient; TV requires the same user to have watched the complete season. Music never participates. YouTube downloads retain their separate shared-interest cleanup rules.

A TV season needs fresh confirmed TMDB series metadata, an exact complete set of confirmed episode mappings, known past air dates, and evidence that the season finished: an ended/cancelled series, a later aired season, or a provider-marked finale. Missing, stale, complex or uncertain mappings block cleanup. Specials are excluded by default. Metadata freshness is seven days; dates must precede the current UTC date.

Eligible media enters the administrator's retention queue. Keep protects the logical media and its descendants. Cancel suppresses the current eligibility snapshot. Delete now bypasses the grace period but still checks protection, activity, file generations and ownership. Changing the policy, watched state or file generation requires a fresh grace period. Routine progress reports and favorites do not restart it. Automatic unmanaged deletion requires explicit opt-in on the library root; otherwise an administrator must execute the pending action.

Deletion uses the same durable operation coordinator as manual media operations. Manager reconciliation, captured target sets, file fingerprints, Keep, active playback and watched eligibility are checked before execution. Interrupted or uncertain destructive commands are never automatically replayed. Failed candidates remain visible with their error.

Radarr and Sonarr keep the title entry, unmonitor the deleted unit, remove files through their APIs, and add a list exclusion. Sonarr uses captured episode IDs, never inferred season/episode numbers, and leaves future-season episodes untouched. Reacquisition reuses the manager entry, restores the captured episode monitoring, reverses only exclusions that Thelxinoe created, and searches again. An existing administrator exclusion is preserved. API routes differ: [Radarr exclusions](https://radarr.video/docs/api/) and [Sonarr import-list exclusions](https://github.com/Sonarr/Sonarr/blob/develop/src/Sonarr.Api.V3/ImportLists/ImportListExclusionController.cs).

## Validation

`cargo test -p thelxinoe-server managers::retention` covers grace and root opt-in, real temporary-file deletion, no replay, watched transitions, Keep, activity, replacement generations, uncertain ownership, incomplete/airing/stale/special seasons, Music exclusion, one user's complete watched set, exact episode remonitoring and preservation of foreign exclusions.

`node scripts/test-retention.mjs` is destructive and restricted to the generated Matrix `FIXTURE2` media in `compose.acquisition.test.yaml`. It passed against real Radarr through HTTPS: Keep, deletion, unmonitoring, list exclusion, same-entry reacquisition and exclusion reversal, plus browser settings/queue rendering. It restores the original policy afterward. Evidence is in `.local/retention-result.json` and `.local/retention.png`. Reacquisition verifies that the manager accepts a new search; it does not claim another completed download. Full real-season deletion remains in the release compatibility matrix; exact episode selection and provider completeness are covered by deterministic fixtures.

## Retention

Automatic retention is disabled by default.

Movies are deleted individually. TV is deleted by complete season. Music never participates in watched-state retention.

Admins choose separate Movie and TV retention-trigger users. Any configured trigger user is sufficient. For TV, that user must have watched all locally available regular episodes in a complete season.

Currently airing or uncertain seasons never become eligible. Season 0 and Specials are excluded by default.

Eligible units wait through a configurable grace period. A shared admin Keep flag always protects Movies/Shows.

Managed cleanup keeps the Radarr/Sonarr entity, deletes media through the manager, unmonitors the cleaned unit, and applies import-list exclusion where needed to prevent automatic re-add. Re-requesting the title reuses the existing manager entry, reverses the retention exclusion as needed, remonitors, and searches again.

YouTube uses a separate rule. A shared download becomes eligible only when no user keeps it through a watchlist or explicit keep/pin reference.

Pending retention actions are visible to admins and can be kept, cancelled, or executed immediately.

All destructive media work uses one server-wide media-operation coordinator. A durable operation records the logical target and the expected generation of every concrete file it intends to affect rather than treating a path as the target. Before execution it acquires the relevant local operation lease, waits for conflicting Thelxinoe playback/transcode/scan/probe/download/import work, refreshes manager ownership, resolves the current concrete files, and rechecks Keep/protection and retention eligibility. If a path now refers to a different file generation, ownership changed, or the target set changed, the operation aborts and returns to reconciliation rather than deleting the new state.

For managed media, the manager API remains the mutation authority and its current activity/state is part of the adapter's final precondition check. Thelxinoe then reconciles the filesystem and manager state before declaring the destructive job complete.

Thelxinoe can coordinate its own work and the managers it integrates with, but it cannot lock an arbitrary external process that writes directly into a library root. Automatic direct filesystem deletion of confirmed-unmanaged media is therefore opt-in per library root. When that option is off, automatic retention may create a pending candidate but an administrator must execute the direct deletion.
