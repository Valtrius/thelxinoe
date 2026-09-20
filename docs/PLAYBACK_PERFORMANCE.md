# Online playback startup

Measured on 2026-09-20 with the local Docker development server, real public provider streams, and Chromium over the HTTPS proxy. These are small live-network samples, not latency guarantees. MPV was not remeasured; it uses the same server extraction and HLS preparation path.

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

## YouTube experiment

The managed yt-dlp 2026.08.19 standalone executable was compared with the matching Python package and default dependencies installed only into a temporary benchmark directory. Both used the managed Deno executable, public access, no configuration/plugins/cookies/cache, and no remote JavaScript component downloads. Two videos were extracted three times per method, alternating method order.

The first batch measured CLI/API medians of 3.15/1.08 seconds. A second batch explicitly checked for supported H.264 video and audio formats and measured 3.13/2.00 seconds; all six attempts per method succeeded. Hashing the two managed executables took another 0.61 seconds in the second batch. Network/session variability therefore matters: the experiment supports a benefit, but not a fixed two-second saving.

The Python API experiment is not enabled in production. It requires a managed Python package snapshot and update/version handling alongside the existing standalone executable bundle. Production YouTube extraction continues to use the verified CLI; it benefits from the shorter continuous online HLS segments. Wholphin uses a separate complete VOD timeline for YouTube seeking, with six-second segments prepared in bounded windows over a shared upstream connection.

## Reproduction

- `scripts/benchmark-online-startup.py`: run inside the server image with JSON containing `{"twitch":[{"login":"public-channel"}]}`. Compares CLI/resident extraction and the three HLS variants. It prints timings, never signed URLs, and cleans up its FFmpeg processes and temporary directories.
- `scripts/benchmark-youtube-startup.py`: run inside the server image with `{"youtube":[{"id":"public-video-id"}]}`. Install the matching `yt-dlp[default]` package into `/tmp/thelxinoe-benchmark-packages` first. The script refuses mismatched versions or ambiguous installed executable bundles. Remove that temporary package directory after testing.
- `scripts/benchmark-online-player.mjs`: set `THELXINOE_BENCHMARK_URL` to an explicitly selected development server with linked Twitch, then run it with Node. Optional `THELXINOE_BENCHMARK_USER` and `THELXINOE_BENCHMARK_PASSWORD` override the development fixture account. `THELXINOE_BENCHMARK_OUTPUT` selects the JSON output; `THELXINOE_BENCHMARK_HOLD_SECONDS=20` adds a sustained playback check on the first channel. Run against an idle instance after the extraction cache has expired. Each run logs out only its own test session.

Local evidence is retained under `.local/startup-benchmark/`: `twitch-before.jsonl`, `player-before.json`, `player-after-recovery.json`, `youtube.jsonl`, `youtube-validated.jsonl`, and `youtube-seek.json`. These files are intentionally untracked. Signed provider URLs and authentication values are excluded from the results.
