# Test commands

Run from the repository root after `npm ci`. Install Rust 1.96+, Node 24+, FFmpeg/FFprobe, Docker (Linux containers) and Playwright Chromium (`npx playwright install chromium`).

Follow the [testing rules in AGENTS.md](../AGENTS.md#testing). Prefer complete application workflows; extend an existing scenario when possible.

```sh
npm run validate
npm run ci
npm run test:ui:layout
npm run test:ui:player
npm run test:service-access
```

[Scripts](../package.json) and [CI runner](../scripts/ci.mjs) define the checks. On Windows, `ci:server` runs Linux checks via [Dockerfile.verify](../scripts/Dockerfile.verify); `ci:desktop` runs native checks. `ci:containers` deletes its disposable `thelxinoe-test` and `thelxinoe-playback` containers/volumes before and after running. UI suites intercept API requests and use no server account; player tests need FFmpeg.

## Test artifacts

Native Radarr/Sonarr/Lidarr/Prowlarr/Bazarr/NZBGet checks use fresh Docker storage and save safe results and
screenshots in `test-results/service-access`. The [runner](../scripts/test-service-access.mjs)
builds its images; pass `-- --built` only after building the current server and
controller as `thelxinoe-service-{server,controller}:local`. See the
[failure contract](../tests/service-access.md). Native browser traces are omitted
because service responses contain API keys.

On Windows with Linux Docker available, `npm run test:service-access -- --desktop`
also builds an isolated Tauri profile and verifies the OS browser handoff from a
real keyring session. It opens the fixture in the default browser. The regular
container CI lane runs the portable service/browser checks.

Playwright saves an HTML report under `playwright-report/{e2e,layout,player}`
and machine-readable results and traces under `test-results/{e2e,layout,player}`,
including successful runs. Open a report with, for example,
`npx playwright show-report playwright-report/layout`. CI uploads these artifacts
even when a test fails. The standalone Docker scripts save their fixture
results and screenshots under `.local/` as specified in each script.

Run `node scripts/test-container-identity.mjs` to build and verify default/custom
UID/GID deployments with real Linux file permissions, managed-service updates,
adoption, product validation and saved Compose recreation. It removes its
disposable containers/volumes and writes `.local/container-identity-result.json`.
Use `--built` with current `thelxinoe-service-{server,controller}:local` images.

## Browser fixtures

All integration fixtures below mutate accounts, files or containers. Use their dedicated deployments. The scripts define ports, credentials and assertions; do not point them at a personal instance. Generated media and results stay under `.local`.

```powershell
node scripts/fixtures.mjs
docker compose build
docker compose -f compose.test.yaml up -d --wait
$env:THELXINOE_PROXY_TEST = '1'
$env:THELXINOE_TEST_URL = 'https://localhost:9443'
npm run test:e2e
# Add appearance checks to this same disposable catalog fixture:
$env:THELXINOE_UI_TEST = '1'
npm run test:e2e
```

First-run tests: start `npm run dev:fresh`, set `THELXINOE_TEST_URL` to its displayed URL, clear `THELXINOE_PROXY_TEST`/`THELXINOE_UI_TEST`, and run `npm run test:e2e`. Provider interaction tests use a separate fresh instance on port 19487 with `THELXINOE_INTERACTION_TEST=1` and `npx playwright test --workers=1`. Windows bind-mount notification checks can take eleven minutes while waiting for periodic reconciliation.

For playback, generate `node scripts/playback-fixtures.mjs`, set `THELXINOE_TEST_HTTP_PORT=18686`, `THELXINOE_TEST_HTTPS_PORT=20443`, `THELXINOE_TEST_SUBNET=172.31.252.0/24`, then run:

```sh
docker compose -p thelxinoe-playback -f compose.test.yaml up -d --wait
node scripts/test-playback.mjs
node scripts/test-playback-tracks.mjs
node scripts/test-user-media.mjs
```

| Area                              | Fixture / executable checks                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Shared mounts and managed stack   | [media mounts](../scripts/test-media-mounts.mjs), [stack](../scripts/test-managed-stack.mjs), [wiring](../scripts/test-managed-wiring.mjs), [UI](../scripts/test-managed-ui.mjs), [adoption](../scripts/test-managed-adoption.mjs)                                                                                                                                                                                                                                                                     |
| Acquisition and external services | [Compose](../compose.acquisition.test.yaml), [fixture generator](../scripts/acquisition-fixtures.mjs), [acquisition](../scripts/test-acquisition.mjs), [download setup](../scripts/configure-acquisition-downloads.mjs), [downloads/import](../scripts/test-acquisition-downloads.mjs), [files](../scripts/test-manager-files.mjs), [TV/music](../scripts/test-manager-tv-music.mjs), [support services](../scripts/test-support-services.mjs), [connections](../scripts/test-service-connections.mjs) |
| Service updates                   | [preflights](../scripts/test-service-preflights.mjs), [commit](../scripts/test-service-update-commit.mjs), [rollback](../scripts/test-service-update-rollback.mjs), [deployment recreation](../scripts/test-deployment-compose.mjs)                                                                                                                                                                                                                                                                    |
| Retention and segments            | [retention](../scripts/test-retention.mjs), [segment fixtures](../scripts/generate-segment-fixtures.mjs), [segments](../scripts/test-segments.mjs), [native segments](../scripts/test-native-segments.mjs)                                                                                                                                                                                                                                                                                             |
| Administration/backups            | [Compose](../compose.operations.test.yaml), [admin UI](../scripts/test-admin-ui.mjs), [backup/restore](../scripts/test-operations.mjs), [interruption recovery](../scripts/test-backup-interruption.mjs)                                                                                                                                                                                                                                                                                               |

## Providers and clients

Use [compose.online.yaml](../compose.online.yaml) for live accounts, with setup on each provider page. [test-youtube.mjs](../scripts/test-youtube.mjs) replaces Google credentials and clears fixture data: run it only on a separate disposable `compose.test.yaml` project at HTTP 18888 / HTTPS 22443 / subnet `172.31.254.0/24`, before linking real accounts.

```sh
node scripts/test-youtube-playback.mjs
node scripts/test-youtube-stream.mjs
node scripts/test-twitch-playback.mjs
node scripts/test-kick-playback.mjs CURRENTLY_LIVE_CHANNEL
```

Live scripts need configured applications, a linked YouTube/Twitch account and available public media. Browser decoding checks do not establish long-running live/ad behavior. Startup measurements: [Twitch browser](../scripts/benchmark-online-player.mjs), [YouTube browser](../scripts/benchmark-youtube-player.mjs) (pass watchlist video IDs), [native](../scripts/benchmark-native-player.mjs), [Streamlink](../scripts/benchmark-online-startup.py), [YouTube extraction](../scripts/benchmark-youtube-startup.py). Select an idle development instance with `THELXINOE_BENCHMARK_URL`; native measurements also need `THELXINOE_BENCHMARK_CDP`.

Windows: build with `npm run build:desktop`, set `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223` and a separate `WEBVIEW2_USER_DATA_FOLDER`, then launch `target/release/thelxinoe-desktop.exe`. Run [test-desktop.mjs](../scripts/test-desktop.mjs) against the proxy fixture or [test-native-playback.mjs](../scripts/test-native-playback.mjs) against the playback fixture. Close the test app afterward. Upstream MPV/plugin qualification: `cargo test -p thelxinoe-desktop qualify_upstream_packages -- --ignored --nocapture`.

TV: generate playback fixtures, then `node scripts/tv-fixtures.mjs`. Start `compose.test.yaml` as `thelxinoe-compat` with HTTP 18787 / HTTPS 21443 / subnet `172.31.253.0/24`, then `node scripts/test-jellyfin.mjs --tv`. An Android TV emulator reaches the host at `http://10.0.2.2:18787`; [test-tv-quick-connect.mjs](../scripts/test-tv-quick-connect.mjs) approves its displayed code. Repeat login/Quick Connect, browsing, direct/remux/transcode, seek/resume/subtitles, music, playlists and online playback in actual clients. Protocol tests alone do not establish client compatibility; check UDP discovery on a real LAN.

## Release checks

Use the [local update lab and automated qualification](UPDATES.md) for server and
Windows updates. The automated suites are included in CI and retain inspectable
evidence. Continue to qualify the applicable real playback, TV and provider
workflows above when changing their protocols.
The optional `node scripts/test-pwa-remote.mjs` checks PWA installation, offline
reconnect and remote HTTPS playback against the existing `compose.test.yaml`
fixture (default `https://localhost:9443`).

## Why isolated coverage remains

These are existing exceptions, not permission to add tests after implementation.
Before adding an exception, enumerate its failure modes and establish why the
application workflows cannot reliably exercise them.

| Area                                    | Failure missed by the current application workflows                                                                                                                                                    |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Database, jobs, event writes            | Concurrent readers/writers, queue overload, panic or event-write failure must preserve transaction boundaries and accepted work.                                                                       |
| Authentication and provider sessions    | Cross-user credentials, forged transports, quota races, and late refresh/authorization replies must not bypass revocation or restore deleted data. Live-provider smoke tests cannot force these races. |
| Managed services and retention          | Lost upstream replies, ambiguous Docker state, foreign ownership, default quality filtering, changed file generations, and exact deletion eligibility need controlled fault injection.                 |
| Archives, releases, paths, subprocesses | Malformed packages, traversal, symlinks, corrupt signatures, oversized/truncated output, and child-process escapes are absent from normal fixture inputs.                                              |
| Playback, catalog, segments, statistics | Conditional client capabilities, replacement/eviction, duplicate music recordings across albums, false intro matches, suspension, and DST boundaries are absent from the ordinary media fixtures.      |
| Desktop tool selection and updates      | Concurrent saves/downloads, revoked selections, release replacement, rate-limit recovery, leases, and cleanup are not exercised by the native playback smoke tests.                                    |
| MPV configuration                       | Profiles, negated options, duplicate assignments, unsaved drafts, and imported plugins can corrupt saved configuration; the browser suites do not run the native configuration editor.                 |
| Frontend transport and defensive inputs | Reconnect cursor ordering, failed discovery retry, malformed provider links/progress, and unsafe service URLs are not supplied by the existing browser fixtures.                                       |

Constant tables, field-copy assertions, fake-DOM geometry, mock call sequences,
source-layout checks, and happy paths already exercised through the application
do not justify another test.
