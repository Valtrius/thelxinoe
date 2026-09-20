# Online playback startup

Measured on 2026-09-20 with the local Docker development server, real public provider streams, Chromium over the HTTPS proxy, and an isolated Windows desktop profile. These are small live-network samples, not latency guarantees.

## Twitch

Three followed live channels were tested. CLI and resident extraction alternated order across three rounds. HLS variants alternated order across two rounds, using the same extracted source for each set. All extraction and HLS attempts succeeded.

| Stage                                   |                    Previous median |                    Candidate median | Samples per variant |
| --------------------------------------- | ---------------------------------: | ----------------------------------: | ------------------: |
| Public extraction                       | 1.57 s, new Streamlink CLI process | 0.89 s, resident Streamlink session |                   9 |
| First HLS playlist                      |        3.43 s, six-second segments |         1.29 s, two-second segments |                   6 |
| First HLS playlist with reduced probing |                                  — |         1.49 s, two-second segments |                   6 |

Reduced FFmpeg probing did not improve this sample and was not adopted. The implemented changes are two resident Streamlink workers and two-second segments for continuous online playback. The rolling playlist still covers approximately two minutes. Local media keeps its existing segment settings.

Browser timing starts at the card click and ends at the first decoded frame with an advancing playback position. A second pass reuses the server's one-minute extracted-source cache; it still creates a fresh playback session and FFmpeg pipeline.

| Browser run                                   | Before |  After |
| --------------------------------------------- | -----: | -----: |
| Uncached extraction, median of three channels | 4.89 s | 2.78 s |
| Cached extraction, median of three channels   | 3.34 s | 1.75 s |

The final after run also tested recovery: both resident workers were killed beforehand. The first channel restarted its worker and began playing in 3.77 seconds; the other two started in 2.53 and 2.78 seconds. Twenty seconds of advancing playback then passed. The worker environments contained only isolated home/config/cache/temp paths and locale settings. No application credentials were inherited.

Server/playback tests and Clippy passed. Public YouTube playback still decoded video, sought successfully, and saved resume after the segment change. No tracked Kick channel was live during this verification, so Kick was not revalidated against the provider.

## YouTube extraction

The managed yt-dlp 2026.08.19 standalone executable was compared with the matching Python package and default dependencies installed only into a temporary benchmark directory. Both used the managed Deno executable, public access, no configuration/plugins/cookies/cache, and no remote JavaScript component downloads. Two videos were extracted three times per method, alternating method order.

The first batch measured CLI/API medians of 3.15/1.08 seconds. A second batch explicitly checked for supported H.264 video and audio formats and measured 3.13/2.00 seconds; all six attempts per method succeeded. Hashing the two managed executables took another 0.61 seconds in the second batch. Network/session variability therefore matters: the experiment supports a benefit, but not a fixed two-second saving.

The follow-up implementation uses two resident workers and a verified official zipimport asset from the same release as the managed CLI. This asset includes the matching EJS components, so updates remain independent of the server image without runtime pip installation. See the [upstream EJS packaging documentation](https://github.com/yt-dlp/yt-dlp/wiki/EJS). The CLI remains available while the matching workers are being prepared and on hosts without the container's Python environment.

Two further batches each compared six CLI extractions with six resident extractions, alternating order over the same two videos. All returned usable video and audio formats. The temporary official-asset benchmark measured medians of **3.24 seconds CLI / 1.10 seconds resident**; the committed benchmark using the installed managed module measured **3.15 / 1.04 seconds**. These numbers cover extraction only, excluding server verification, stream preparation and player buffering. The latter batch measured another 0.64 seconds to hash the CLI and Deno executables; resident requests verify the much smaller module and Deno instead.

Live verification killed both resident workers before submitting two simultaneous extraction requests. Both requests recovered and returned formats in 3.60 and 4.60 seconds; a second concurrent round completed in 2.85 and 1.94 seconds. Both worker environments contained only isolated home/config/cache/temp paths and locale settings. Four Rust regression tests cover framing, version changes, module integrity and manifest pinning; three offline Python tests cover state reuse, provider errors, output limits, public-only options and version mismatch. Server unit tests, playback/Jellyfin integration tests and Clippy passed.

Wholphin continues to use a separate complete VOD timeline for YouTube seeking, with six-second segments prepared in bounded windows over a shared upstream connection. Browser playback keeps its existing HLS path. Both use the resident extractor when it is ready.

## Reproduction

- `scripts/benchmark-online-startup.py`: run inside the server image with JSON containing `{"twitch":[{"login":"public-channel"}]}`. Compares CLI/resident extraction and the three HLS variants. It prints timings, never signed URLs, and cleans up its FFmpeg processes and temporary directories.
- `scripts/benchmark-youtube-startup.py`: run inside the server image with `{"youtube":[{"id":"public-video-id"}]}` after the server prepares its resident module. It uses the verified managed module, requires no extra installation, and refuses mismatched versions or ambiguous installed executable bundles.
- `scripts/benchmark-online-player.mjs`: set `THELXINOE_BENCHMARK_URL` to an explicitly selected development server with linked Twitch, then run it with Node. Optional `THELXINOE_BENCHMARK_USER` and `THELXINOE_BENCHMARK_PASSWORD` override the development fixture account. `THELXINOE_BENCHMARK_OUTPUT` selects the JSON output; `THELXINOE_BENCHMARK_HOLD_SECONDS=20` adds a sustained playback check on the first channel. Run against an idle instance after the extraction cache has expired. Each run logs out only its own test session.
- `scripts/benchmark-native-player.mjs`: set `THELXINOE_BENCHMARK_CDP` to an isolated desktop's WebView2 debugging address and `THELXINOE_BENCHMARK_URL` to its connected development server. Sign in first and stop existing playback. Optional `THELXINOE_BENCHMARK_VIDEO` fixes the public YouTube fixture; `THELXINOE_BENCHMARK_OUTPUT` selects the result file. The script retains the selected MPV configuration and plugins, starts and stops its test playback, and reports preparation and readiness separately. It does not change the connected server or sign out the desktop.

Local evidence is retained under `.local/startup-benchmark/`: `twitch-before.jsonl`, `player-before.json`, `player-after-recovery.json`, `youtube.jsonl`, `youtube-validated.jsonl`, and `youtube-seek.json`. These files are intentionally untracked. Signed provider URLs and authentication values are excluded from the results.

## Windows MPV follow-up

The initial native YouTube path opened MPV immediately, then waited for extraction and an HLS conversion pipeline. On a recorded video, even a cached source took about 4.6 seconds to prepare on the server. MPV needed roughly another half second. Reducing MPV's probing did not materially improve the sample.

The desktop now advertises support for separate online video/audio files. Auto quality can relay seekable YouTube sources through the server to MPV, preserving the selected H.264 video up to 1080p and its audio. Extraction, scoped grants, progress and access checks remain on the server. This avoids the conversion and segment wait; Twitch/Kick, bitrate-limited playback and unsupported source formats keep the existing HLS path. The YouTube extraction cache remains five minutes, and an uncached launch still needs yt-dlp, now through a resident worker when ready.

| Native YouTube launch             | Previous HLS | Server relay |
| --------------------------------- | -----------: | -----------: |
| First launch, uncached extraction |       9.47 s |       4.85 s |
| Two repeat launches, median       |       5.20 s |       0.86 s |

These measurements invoke the desktop playback command with the same video at position zero and use MPV's readiness report as the endpoint, then confirm advancing playback separately. The app reports readiness at 500 ms intervals. The committed benchmark also waits for a positive playback position before ending its timer; a subsequent run with that stricter endpoint measured 5.81 seconds initially and 1.84 seconds for repeat launches. Full card-click checks before the change reproduced a 9.77-second initial launch. A web run immediately afterward reused the extraction cache, so comparing those two launches would overstate the desktop overhead.

Clicking the first undownloaded watchlist video in the rebuilt desktop, resuming around 1:33, took 5.55 seconds with uncached extraction and 0.84 seconds on the next click.

Live native validation confirmed selected video and external audio tracks, decoded audio, more than two minutes of continuous playback, paused and backward seeking, a seek after grant renewal, saved progress and resume. Cached Twitch native launches remained around two seconds. A separate initial-buffer experiment improved YouTube less and did not help the tested seek; neither its FFmpeg upgrade nor its pacing changes were retained.

Native evidence is retained under `.local/startup-native/`: `before.json`, `relay-after.json`, `verified-native.json`, `paired.json`, `probe.json`, `watchlist-click.json`, `relay-protocol.json`, and `relay-validation.json`. The isolated profile used a copy of the existing MPV configuration and the same executable; the user's running desktop and saved plugin settings were left intact.

### With resident YouTube extraction

The first uncached launch after deploying resident workers took **4.31 seconds** to reach readiness with positive playback position, including **2.77 seconds** of server preparation. Repeat launches took 1.84 seconds. This uses the stricter endpoint of the earlier 5.81-second CLI run; network conditions were not controlled between runs. Two further uncached videos passed at 4.70 and 4.45 seconds, with repeat launches around 1.84 seconds. A third reached readiness at 4.66 seconds but failed the short sustained-playback assertion: a follow-up trace showed an approximately 1.5-second initial buffering pause, then continuous advancement through 12 seconds. Resident extraction reduces preparation work; it does not eliminate CDN/player buffering variability. A browser check over HTTPS also passed streaming without a download, decoded frames, seeking and saved resume.

Evidence is under `.local/warm-youtube/`: `zip-benchmark.jsonl`, `managed-benchmark.jsonl`, `native-first.json`, `native-uncached.json`, `recovery.json`, `stall.json` and `web-seek.json`. These intentionally untracked files contain timing and validation results, without credentials or signed stream URLs.
