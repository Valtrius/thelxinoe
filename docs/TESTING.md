# Local verification

```powershell
npm run validate
npm run desktop:build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

On Linux, omit `--workspace` from Clippy to exclude the Windows desktop shell. Catalog tests require FFprobe. Generated fixtures require FFmpeg. CI installs those prerequisites explicitly.

Windows hosts can also run the complete Linux Rust checks in Docker:

```powershell
docker build -f scripts/Dockerfile.verify -t thelxinoe-verified:local .
```

This runs formatting, Clippy and all default workspace tests with FFmpeg/FFprobe installed, including Linux-only controller locking, archive permissions and process behavior. The full acceptance matrix and local evidence are in [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).

## Browser and reverse proxy

```powershell
node scripts/fixtures.mjs
docker compose build
docker compose -f compose.test.yaml up -d --wait
$env:THELXINOE_PROXY_TEST = '1'
$env:THELXINOE_TEST_URL = 'https://localhost:9443'
npm run test:e2e
```

The proxy project runs first and creates its test administrator on a fresh database. The catalog project indexes generated video, multi-episode TV, music tags and a local trailer. Checks cover secure cookies, Origin rejection, event replay, device revocation, stable identities and manual corrections. Only the test browser ignores the local Caddy certificate's trust error.

Docker Desktop may not forward Windows file notifications into Linux bind mounts. On Windows, the catalog test allows the server's five-minute reconciliation fallback for file creation and removal; this portion can take up to eleven minutes. Native notification tests keep the shorter deadline.

For the first-run page on an untouched main deployment, clear those two environment variables and run `npm run test:e2e`.

### Appearance and media motion

After creating the disposable proxy/catalog fixture above, run the appearance suite against that same explicit test URL:

```powershell
$env:THELXINOE_UI_TEST = '1'
npm run test:e2e
```

The suite changes the test user's theme and density, samples intermediate sidebar animation frames, checks Ctrl+wheel sizing and mobile overflow, and uses real delayed HTTP requests to check concurrent optimistic watchlist additions. It expects the fixture administrator and at least one indexed movie. It adds two synthetic video IDs to that user's watchlist, so use a disposable test deployment. Screenshots are written under `.local/ui-validation`.

Native checks should use an isolated Tauri identifier and WebView profile. Verify that theme controls appear only in the title bar, MPV appears only in native Settings, light/dark selection persists through the native transport, and maximize/restore/close work. Test ordinary production configuration as well as release fixtures: the updater plugin needs its bootstrap configuration even when no release channel is set.

## Browser playback

Use the separate playback fixture deployment after building the server image:

```powershell
node scripts/playback-fixtures.mjs
$env:THELXINOE_TEST_HTTP_PORT = '18686'
$env:THELXINOE_TEST_HTTPS_PORT = '20443'
$env:THELXINOE_TEST_SUBNET = '172.31.252.0/24'
docker compose -p thelxinoe-playback -f compose.test.yaml up -d --wait
node scripts/test-playback.mjs
node scripts/test-playback-tracks.mjs
```

The first script checks real Chromium decoding of direct, remux and converted streams, seeks, range/HEAD/416 behavior, sidecar subtitles, grant revocation, two users, out-of-order reports, 90% watched inference and edition resume. It captures the application's actual Web Audio scheduling, then renders the buffers across their join to measure sample continuity and ReplayGain. The second verifies saved language preferences, embedded subtitles and the selected audio's actual 880 Hz signal. A Rust integration test replaces a real generated media file, rescans and reopens the database to verify logical resume and rejection of the old file generation.

Browser gapless delivery uses up to two decoded FLAC/PCM tracks (64 MiB encoded and 128 MiB decoded per track, up to ten minutes each). Other formats and explicit conversion preferences use streaming playback with ReplayGain. Conversion uses four bounded FFmpeg slots, a rolling HLS window, a 2 GiB cache ceiling and idle cleanup; direct streams do not consume conversion slots. Playback grants expire after two minutes without an authenticated heartbeat and remain tied to their parent login session. The supported v1 browser matrix is Chromium, including mobile emulation and the installed Windows WebView2 shell.

## Windows desktop runtime

With the test deployment initialized:

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9223'
$env:WEBVIEW2_USER_DATA_FOLDER = "$PWD\.local\desktop-test-webview"
Start-Process -FilePath "$PWD\target\release\thelxinoe-desktop.exe" -WindowStyle Hidden
node scripts/test-desktop.mjs
```

This controls the real bundled Svelte page inside WebView2. It tests device login, persistence in Windows Credential Manager, absence of credentials in browser storage, catalog browsing, server restart/reconnect, and revocation. It restores the desktop server address to `http://127.0.0.1:8484`. Close the test app afterward; remote debugging is enabled only by these environment variables. The script restarts only the isolated test server.

## User media state

With the isolated playback deployment running:

```powershell
node scripts/test-user-media.mjs
```

This exercises separate users' Favorites and Watch Later, owner-controlled shared playlists, independent playlist favorites, actual playlist playback, queue restoration after reload, separate browser-client queues, display timezone and history permissions through HTTPS. Rust integration tests additionally cover stale revisions, late progress from an earlier queued track, seeks not inflating viewing time, the correct edition in Continue Watching, and history surviving device revocation.

Playback time is accumulated from observed position advances bounded by elapsed reporting time. Each report can add at most 30 seconds; large seek jumps are excluded. Timestamps are stored in UTC. History displays the user's selected timezone; its date filters explicitly use UTC.

## Windows MPV playback

Start the isolated playback Compose deployment and generate its catalog using the local playback instructions above. Launch the bundled Windows application with the same WebView2 debug settings used by the desktop test, then run:

```powershell
node scripts/test-native-playback.mjs
```

The test uses the playback fixture server on port 18686. It installs the official Windows x64 MPV build if needed, verifies its upstream SHA-256 digest, exercises native video output, pause, seek, desktop page reload, server resume, and HLS conversion. It temporarily selects MPV's PCM output for the two FLAC fixtures, then checks the sample count and amplitude across the track boundary. Configuration, playback preferences and the server address are restored afterward. Results are written under `.local`.

MPV configuration and Lua plugins live in Thelxinoe's application data directory. The application supports a managed installation or a supplied MPV executable, without depending on YouTwitch's tool directory. Playback URLs travel over a random named pipe; account credentials and media URLs are absent from MPV process arguments. Closing the desktop stops its player and reports the last position to the server.

## Android TV preparation

Android Studio's SDK is at `$env:LOCALAPPDATA\Android\Sdk`. The installed image is `system-images;android-34;android-tv;x86`. The project AVD uses Windows Hypervisor Platform and a software GPU. The other existing AVD was left intact.

```powershell
$env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk"
$env:ANDROID_AVD_HOME = "$PWD\.local\android-avd"
Start-Process -FilePath "$env:ANDROID_HOME\emulator\emulator.exe" -ArgumentList '-avd Thelxinoe_TV -no-window -no-audio -no-boot-anim -no-snapshot -gpu swiftshader_indirect -port 5580' -WindowStyle Hidden
& "$env:ANDROID_HOME\platform-tools\adb.exe" -s emulator-5580 shell getprop sys.boot_completed
& "$env:ANDROID_HOME\platform-tools\adb.exe" -s emulator-5580 shell monkey -p com.github.damontecres.wholphin -c android.intent.category.LEANBACK_LAUNCHER 1
```

ADB serial: `emulator-5580`. The emulator reaches the compatibility server at `http://10.0.2.2:18787`. Wholphin's ARMv7 APK works on this image's translation layer; its x86_64 APK does not match the image ABI.

```powershell
node scripts/tv-fixtures.mjs
$env:THELXINOE_TEST_HTTP_PORT = '18787'
$env:THELXINOE_TEST_HTTPS_PORT = '21443'
$env:THELXINOE_TEST_SUBNET = '172.31.253.0/24'
$env:THELXINOE_TEST_LOG = 'thelxinoe=info,thelxinoe_server::jellyfin=debug'
docker compose -p thelxinoe-compat -f compose.test.yaml up -d --wait
node scripts/test-jellyfin.mjs --tv
```

Generate the standard playback fixtures first. TV fixtures add 90-second timestamped remux/transcode sources, timed subtitle cues, and an HDR conversion source. Recreating fixture files changes their generation and invalidates existing playback sessions. See [the client matrix](JELLYFIN.md) for actual TV checks, as the protocol script alone cannot establish client compatibility.

Wholphin 1.0.8 ARMv7 APK SHA-256: `7a7a031104f42a8314e3deed4febb670b515687ac500026f59a997dae772d950`. Source: [official Wholphin releases](https://github.com/damontecres/Wholphin/releases).

## YouTube account and feed validation

Use a dedicated fixture deployment; this test replaces its Google application configuration with fictional credentials and clears the fixture account's YouTube data. It refuses to proceed if that account is connected. Do not run it after linking a real account for live validation.

```powershell
$env:THELXINOE_TEST_HTTP_PORT = '18888'
$env:THELXINOE_TEST_HTTPS_PORT = '22443'
$env:THELXINOE_TEST_SUBNET = '172.31.254.0/24'
docker compose -p thelxinoe-online -f compose.test.yaml up -d --wait
node scripts/test-youtube.mjs
```

The script exercises the real browser UI and backend through Caddy HTTPS. It serves a fictional Google authorization page in the browser to check the cross-site callback: its Lax cookie is present, the Strict login cookie is absent, and normal authenticated browsing resumes afterward. It verifies PKCE, replay rejection, admin-only settings, concurrent immediate watchlist placeholders, private pins/watched state, and disconnect preserving saved videos. Evidence: `.local/youtube-result.json`, `.local/youtube-watchlist.png`, `.local/youtube-settings.png`.

Rust tests use a bounded local provider stub to check successful code exchange, encrypted storage, concurrent refresh, provider revocation, fair shared quota reservations, two-user subscription paging, persisted restart recovery, metadata isolation, and late responses after disconnect/deletion. No production endpoint override is exposed for these stubs.

The read-only helper below checks whether the existing YouTwitch Google application reaches an interactive authorization page for the proposed callback. It never prints credentials, changes YouTwitch or signs in. Reaching sign-in does not prove that consent or the token exchange will succeed; those require the account owner.

```powershell
cargo run -p thelxinoe-desktop --example check-google-callback -- https://localhost:22443/api/v1/online/youtube/callback
```

## Public YouTube playback validation

After linking a real account on the isolated online deployment and installing the managed tools:

```powershell
node scripts/test-youtube-playback.mjs
node scripts/test-youtube-stream.mjs
```

The first test uses a retained public fixture and verifies decoded browser frames, seeking and server resume through HTTPS. The second selects a public VOD from the linked feed and verifies immediate streaming without downloading. These scripts do not replace application credentials. Live checks use a currently live public feed item; they verify decoded frames, absence of VOD seeking, and unchanged watched state. Native checks exercise the rebuilt Windows application through its MPV IPC and verify rapid VOD-to-live transitions, pause controls and server history. Private account-specific fixtures and sanitized results stay under `.local`.

## Private provider import

The offline helpers use an existing master key, never display credentials, and require the destination server to be stopped. For the main Windows Docker bind mount:

```powershell
docker compose stop server
cargo run -p thelxinoe-desktop --example import-youtwitch -- .local/docker/server
# After placing the Thelxinoe TMDB token in the ignored local input file:
cargo run -p thelxinoe-desktop --example import-tmdb -- .local/docker/server .local/tmdb-token .local/musicbrainz-contact
docker compose up -d --wait
```

Alternatively, an administrator can save metadata settings through the application. A configured token is never returned by the API. No real token or provider-contact address is committed.

## Live metadata validation

This opt-in test uses those private input files against real providers. It creates separate test accounts, provider matches and named volumes, leaving the regular catalog fixtures independent.

```powershell
$env:THELXINOE_TEST_HTTP_PORT = '18585'
$env:THELXINOE_TEST_HTTPS_PORT = '19443'
$env:THELXINOE_TEST_SUBNET = '172.31.251.0/24'
docker compose -p thelxinoe-live -f compose.test.yaml up -d --wait
node scripts/test-live-metadata.mjs
```

It checks real movie/show searches and matches, collection membership, explicit episode mapping, MusicBrainz artist/album matching, downloaded TMDB/CAA images, and refresh preserving a manual correction. Evidence contains public titles and booleans, never credentials.

## Twitch and Kick live validation

After connecting Twitch on the isolated HTTPS online deployment:

```powershell
node scripts/test-twitch-playback.mjs
node scripts/test-kick-playback.mjs starladder
```

The Twitch script selects a followed live channel. The Kick script temporarily tracks the named channel, waits for live metadata, and removes its temporary interest afterward. Choose a currently live public channel; availability changes. Both verify decoded browser frames, pause, hidden VOD seeking, and server history through HTTPS. Native validation uses the production Windows bundle and checks MPV video readiness, pause controls and history. Live test accounts, selected channels and results remain in `.local`.

A read-only helper can list public live Kick channels using the existing YouTwitch application configuration. It prints channel names only:

```powershell
cargo run -p thelxinoe-desktop --example check-kick
```

Linux fixture tests cover device-code/session binding, encrypted tokens, refresh, revoked/late replies, pagination, user isolation, rate limits, tracked-channel generations and live-session cleanup. They run without real account credentials.

## Product release, recovery and native installer

Use a dedicated local registry, publisher key, server state and test application identity. The fixture intentionally migrates to a schema the original release cannot open. Never point it at the main or live-provider deployment.

1. Build the normal Linux images and Windows installer. Tag the Linux images `thelxinoe-server:release-base` and `thelxinoe-controller:release-base`.
2. Run `node scripts/prepare-release-fixture.mjs`. Build both targets from `.local/release-fixture`, tag them `localhost:25000/thelxinoe/server:0.2.0` and `localhost:25000/thelxinoe/controller:0.2.0`, and push to a registry named `thelxinoe-release-registry` bound to `127.0.0.1:25000`.
3. Run `node scripts/prepare-release-test.mjs`. This generates a private test manifest key if absent and signs the candidate image/schema metadata. The controller-only fixture uses placeholder desktop metadata; do not install that artifact.
4. Start `docker compose -f compose.release.test.yaml up -d --wait`. On Linux, pre-create the bind-mounted server/cache/data directories with ownership `10001:10001`. The fixture uses HTTPS 28443, HTTP 19494 and separate state in `.local/releases-v6`.
5. Run the following against that deployment. They use only synthetic accounts/media and the private test channel:

```powershell
$env:THELXINOE_RELEASE_PROJECT = 'thelxinoe-release-v6'
$env:THELXINOE_RELEASE_STATE = '.local/releases-v6'
$env:THELXINOE_RELEASE_ARCHIVE = '1'
node scripts/test-product-release.mjs
node scripts/test-release-interruption.mjs
node scripts/test-release-recreation.mjs
node scripts/test-pwa-remote.mjs
```

The product test checks a rejected live forward migration, successful signed activation, encrypted archive restoration of the earlier release and explicit state recovery. The interruption test stops the registry, kills the controller after forward migration and checks that the original state/server recover offline. The recreation test uses the persisted Compose override with deliberately stale bootstrap image names, actually recreates both containers and checks the accepted images and new generation. Run these in order; they leave recoverable test journals and retained images.

For Windows installer validation, build both fixture versions with the same separate test application identifier (`app.thelxinoe.releasetest`), Tauri signing key and manifest publisher key. Use `THELXINOE_RELEASE_PUBLIC_KEY` at build time for that test key, and `THELXINOE_RELEASE_CA_PEM` only when serving the local publisher with a private TLS CA. The fixture override enables `bundle.createUpdaterArtifacts` and supplies `plugins.updater.pubkey`. Publish the candidate installer as `.local/releases/channel/setup.exe` with its `.sig`, and place the Tauri public key at `.local/releases/tauri.key.pub`. Run `prepare-release-test.mjs` again to assemble and sign the actual desktop artifact metadata. Serve only the channel directory at `https://localhost:29443`; never serve private keys.

Install the base fixture into a separate directory/profile, launch its WebView with a local debugging port 9224, and run `node scripts/test-native-update.mjs`. It checks metadata mismatch, corrupt artifact rejection, actual installation/restart and preserved device authentication. Then run `node scripts/test-native-compatibility.mjs` against that same isolated application to verify the update-required screen and available updater on an incompatible server. Close the test application afterward. Build the production installer in a fresh shell with the normal identity and without either test publisher override.

## PWA and remote quality

`test-pwa-remote.mjs` uses the release fixture above. It creates the test administrator only if setup is required, checks all main sections at 390 pixels, Chromium installability, a public-only cache and offline reconnect. It also verifies HTTPS Range/HLS, cookies/CSRF/API compatibility, remote Auto quality and the update-required screen after an already-open client receives HTTP 426. `THELXINOE_RELEASE_ORIGIN` can override its HTTPS origin. Rust auth tests additionally cover explicit CORS and forwarded-address trust boundaries. The service worker never caches API responses, media or application bundles.
