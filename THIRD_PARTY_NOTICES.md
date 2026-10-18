# Third-party notices

Thelxinoe is licensed under the [MIT License](LICENSE). Its release images and
installers also contain the third-party software below, under the licenses of
their authors.

## Server image

The tools are pinned in [`scripts/server-tools.lock.json`](scripts/server-tools.lock.json),
which records each artifact's download address and SHA-256.

| Component                               | License                                       | Source                                                                                                        |
| --------------------------------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| FFmpeg, BtbN `linux64-gpl` build        | GPL-3.0-or-later                              | [FFmpeg](https://git.ffmpeg.org/ffmpeg.git), [build scripts](https://github.com/BtbN/FFmpeg-Builds)           |
| yt-dlp, including its standalone binary | Unlicense; the binary bundles Python packages | [yt-dlp](https://github.com/yt-dlp/yt-dlp), [nightly builds](https://github.com/yt-dlp/yt-dlp-nightly-builds) |
| Deno                                    | MIT                                           | [denoland/deno](https://github.com/denoland/deno)                                                             |
| Streamlink and its Python dependencies  | BSD-2-Clause and their own licenses           | [streamlink/streamlink](https://github.com/streamlink/streamlink), [PyPI](https://pypi.org/)                  |
| Python 3.11 and Debian base system      | PSF-2.0 and Debian package licenses           | [python:3.11-slim-trixie](https://hub.docker.com/_/python), [Debian](https://sources.debian.org/)             |
| Rust crates compiled into the binaries  | MIT, Apache-2.0 and others                    | [`Cargo.lock`](Cargo.lock)                                                                                    |
| npm packages bundled in the web app     | MIT and others                                | [`pnpm-lock.yaml`](pnpm-lock.yaml)                                                                            |

Each FFmpeg archive identifies the FFmpeg revision it was built from in its name
(for example `n8.1.3-9-g29e619e767`) and the matching BtbN build tag in the lock
file's `notes_url`.

## Windows app

The installer contains the Thelxinoe desktop app, Rust crates and npm packages
listed in the lock files above, and Microsoft's WebView2 bootstrapper. The player
and its tools (mpv, FFmpeg, yt-dlp, Streamlink) are downloaded from their
publishers when you install them in **Settings → MPV**, under their own licenses.

## Source for GPL components

For three years after a release that contains them, the complete corresponding
source of its GPL-licensed components is available on request: open an
[issue](https://github.com/Valtrius/thelxinoe/issues) naming the release, and it
will be provided at no more than the cost of distribution.
