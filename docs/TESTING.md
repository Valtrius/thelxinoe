# Test commands

Run from the repository root after `npm ci`. Install Rust 1.96+, Node 24+, FFmpeg/FFprobe, Docker (Linux containers) and Playwright Chromium (`npx playwright install chromium`).

```sh
npm run validate
npm run ci
npm run test:ui:layout
npm run test:ui:player
```

[Scripts](../package.json) and [CI runner](../scripts/ci.mjs) define the checks. On Windows, `ci:server` runs Linux checks via [Dockerfile.verify](../scripts/Dockerfile.verify); `ci:desktop` runs native checks. `ci:containers` deletes its disposable `thelxinoe-test` and `thelxinoe-playback` containers/volumes before and after running. UI suites intercept API requests and use no server account; player tests need FFmpeg.

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

Live scripts need configured applications, a linked YouTube/Twitch account and available public media. Browser decoding checks do not establish long-running live/ad behavior. Startup measurements: [browser](../scripts/benchmark-online-player.mjs), [native](../scripts/benchmark-native-player.mjs), [Streamlink](../scripts/benchmark-online-startup.py), [YouTube](../scripts/benchmark-youtube-startup.py). Select an idle development instance with `THELXINOE_BENCHMARK_URL`; native measurements also need `THELXINOE_BENCHMARK_CDP`.

Windows: build with `npm run build:desktop`, set `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223` and a separate `WEBVIEW2_USER_DATA_FOLDER`, then launch `target/release/thelxinoe-desktop.exe`. Run [test-desktop.mjs](../scripts/test-desktop.mjs) against the proxy fixture or [test-native-playback.mjs](../scripts/test-native-playback.mjs) against the playback fixture. Close the test app afterward. Upstream MPV/plugin qualification: `cargo test -p thelxinoe-desktop qualify_upstream_packages -- --ignored --nocapture`.

TV: generate playback fixtures, then `node scripts/tv-fixtures.mjs`. Start `compose.test.yaml` as `thelxinoe-compat` with HTTP 18787 / HTTPS 21443 / subnet `172.31.253.0/24`, then `node scripts/test-jellyfin.mjs --tv`. An Android TV emulator reaches the host at `http://10.0.2.2:18787`; [test-tv-quick-connect.mjs](../scripts/test-tv-quick-connect.mjs) approves its displayed code. Repeat login/Quick Connect, browsing, direct/remux/transcode, seek/resume/subtitles, music, playlists and online playback in actual clients. Protocol tests alone do not establish client compatibility; check UDP discovery on a real LAN.

## Release checks

Before publishing, run CI plus the applicable real browser, Windows, TV, provider, service-update and backup/recovery fixtures above. Validate supported clients after protocol changes. Use separate release signing keys, state and desktop identity (`app.thelxinoe.releasetest`).

1. Build normal images and tag them `thelxinoe-server:release-base` and `thelxinoe-controller:release-base`. Run [prepare-release-fixture.mjs](../scripts/prepare-release-fixture.mjs); it injects a schema change into a disposable source copy to test incompatible-version recovery.
2. Build both targets from `.local/release-fixture` and push as `localhost:25000/thelxinoe/server:0.2.0` and `localhost:25000/thelxinoe/controller:0.2.0` to a registry named `thelxinoe-release-registry` bound to `127.0.0.1:25000`.
3. Run [prepare-release-test.mjs](../scripts/prepare-release-test.mjs), then `docker compose -f compose.release.test.yaml up -d --wait`. Linux bind directories need ownership `10001:10001`. Set `THELXINOE_RELEASE_PROJECT=thelxinoe-release-v6`, `THELXINOE_RELEASE_STATE=.local/releases-v6`, `THELXINOE_RELEASE_ARCHIVE=1`.
4. Run these in order:

```sh
node scripts/test-product-release.mjs
node scripts/test-release-interruption.mjs
node scripts/test-release-recreation.mjs
node scripts/test-pwa-remote.mjs
```

For actual Windows update checks, build both fixture versions with that separate identity and matching test signing keys. Supply `THELXINOE_RELEASE_PUBLIC_KEY` at build time; use `THELXINOE_RELEASE_CA_PEM` only for a private publisher CA. Enable updater artifacts/public key via a Tauri override. Put the candidate at `.local/releases/channel/setup.exe` with its `.sig`, and the Tauri public key at `.local/releases/tauri.key.pub`. Rerun `prepare-release-test.mjs`; serve only the channel at `https://localhost:29443`. Install the base in a separate directory/profile, launch with debug port 9224, then run [test-native-update.mjs](../scripts/test-native-update.mjs) and [test-native-compatibility.mjs](../scripts/test-native-compatibility.mjs). Rebuild production in a fresh shell without test key/CA overrides.
