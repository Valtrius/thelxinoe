# Media segments

Segments belong to a concrete file generation. Replacing a file invalidates its timestamps. Manual corrections take precedence, including an empty correction list that suppresses automatic results. Settings offers private Auto, Ask and Ignore choices for Intro, Recap, Credits and Preview; all default to Ask.

The local worker fingerprints the first and last quarter of each episode, capped at ten minutes per window, using FFmpeg Chromaprint. It compares recurring audio with up to eight episodes in the same season, rejects short or low-information matches, and records conservative Intro/Credits boundaries with source and confidence. Fingerprints and work state survive restart. Shared multi-episode files are excluded from automatic analysis. Recap and Preview can be supplied by the external provider or entered manually.

Analysis runs separately at low CPU priority and yields when playback or ready library work appears. Failed analyses are visible in Settings; an administrator can requeue a specific edition from its library details. Generated fixtures demonstrate recurring audio detection; this is not a claim that every show's changing or silent intro will match.

The optional TheIntroDB integration is disabled by default. It sends only confirmed TMDB episode coordinates and runtime to the public provider, validates the returned episode identity and bounds, and keeps external timestamps separate from local/manual results. No account token, file path or audio is sent. External timestamps may describe a different cut; use the per-edition correction UI when necessary.

Browser and Windows MPV controls share the skip policy. Auto seeks once per segment per playback view; seeking back into an already handled segment does not create a seek loop. Jellyfin clients receive normal Media Segments DTOs and apply their own playback preferences. For multiple editions, the compatibility endpoint uses the current device's selected playback file and returns no segments when that choice is ambiguous.

## Validation

- Rust tests cover recurring shifted audio, silence/short-match rejection, provider identity and bounds, private preferences, authorization, manual override/reset, replacement generations and Jellyfin edition/device isolation.
- `node scripts/generate-segment-fixtures.mjs` generates two synthetic episodes with shifted recurring music. `compose.segments.test.yaml` runs the isolated HTTPS test server. `node scripts/test-segments.mjs` checks detected intro/credit bounds, Jellyfin API responses, browser Ask/Auto/Ignore and manual editing. Evidence: `.local/segments-result.json`, `.local/segments-editor.png`.
- `node scripts/test-segments-external.mjs` attaches confirmed public episode metadata to generated media and verifies real TheIntroDB ingestion. Evidence: `.local/segments-external-result.json`. It restores the external-provider setting after testing.
- Wholphin 1.0.8 on the Android TV API 34 emulator requested `/MediaSegments/{id}` successfully, displayed Skip Intro for a generated fixture's 4–25 second segment and advanced to 26 seconds when selected. Evidence: `.local/wholphin-segments-seek.png` and `.local/wholphin-segment-fixtures.json`.
- Windows production bundle builds successfully. The new native segment runtime check is pending launch of the prepared test app; automatic approval review blocked the launch command without a more specific reason.

Protocol references: [Jellyfin Media Segments controller](https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Api/Controllers/MediaSegmentsController.cs), [Chromaprint configuration](https://github.com/acoustid/chromaprint/blob/master/src/fingerprinter_configuration.h), [FFmpeg Chromaprint output](https://github.com/FFmpeg/FFmpeg/blob/master/libavformat/chromaprint.c), [TheIntroDB v3 API client](https://github.com/TheIntroDB/theintrodb-npm).
