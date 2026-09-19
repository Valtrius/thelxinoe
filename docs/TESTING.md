# Local verification

```powershell
npm run validate
npm run desktop:build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

On Linux, omit `--workspace` from Clippy to exclude the Windows desktop shell. Catalog tests require FFprobe. Generated fixtures require FFmpeg. CI installs those prerequisites explicitly.

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

For the first-run page on an untouched main deployment, clear those two environment variables and run `npm run test:e2e`.

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

Browser gapless delivery uses up to two decoded FLAC/PCM tracks (64 MiB encoded and 128 MiB decoded per track, up to ten minutes each). Other formats and explicit conversion preferences use streaming playback with ReplayGain. Conversion uses four bounded FFmpeg slots, a rolling HLS window, a 2 GiB cache ceiling and idle cleanup; direct streams do not consume conversion slots. Playback grants expire after two minutes without an authenticated heartbeat and remain tied to their parent login session. Tests currently exercise Chromium; the final browser matrix belongs to release hardening.

## Windows desktop runtime

With the test deployment initialized:

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9223'
$env:WEBVIEW2_USER_DATA_FOLDER = "$PWD\.local\desktop-test-webview"
Start-Process -FilePath "$PWD\target\release\thelxinoe-desktop.exe" -WindowStyle Hidden
node scripts/test-desktop.mjs
```

This controls the real bundled Svelte page inside WebView2. It tests device login, persistence in Windows Credential Manager, absence of credentials in browser storage, catalog browsing, server restart/reconnect, and revocation. It restores the desktop server address to `http://127.0.0.1:8484`. Close the test app afterward; remote debugging is enabled only by these environment variables. The script restarts only the isolated test server.

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

ADB serial: `emulator-5580`. The emulator reaches host services through `10.0.2.2`. The test server will be `http://10.0.2.2:18484` when the Jellyfin adapter exists. Wholphin's ARMv7 APK works on this image's translation layer; its x86_64 APK does not match the image ABI.

Wholphin 1.0.8 ARMv7 APK SHA-256: `7a7a031104f42a8314e3deed4febb670b515687ac500026f59a997dae772d950`. Source: [official Wholphin releases](https://github.com/damontecres/Wholphin/releases).

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
