# Third-party notices

Thelxinoe is licensed under the [MIT License](LICENSE). Its release images and
installers also contain the third-party software below, under the licenses of
their authors.

## Server image

The tools are pinned in [`scripts/server-tools.lock.json`](scripts/server-tools.lock.json),
which records each artifact's download address and SHA-256.

| Component                               | License                                       | Source                                                                                                        |
| --------------------------------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| FFmpeg, BtbN `linux64-gpl` build        | GPL-3.0-or-later                              | [FFmpeg](https://git.ffmpeg.org/ffmpeg.git), [build scripts](https://github.com/FFmpeg-Builds/FFmpeg-Builds) |
| yt-dlp, including its standalone binary | Unlicense; the binary bundles Python packages | [yt-dlp](https://github.com/yt-dlp/yt-dlp), [nightly builds](https://github.com/yt-dlp/yt-dlp-nightly-builds) |
| Deno                                    | MIT                                           | [denoland/deno](https://github.com/denoland/deno)                                                             |
| Streamlink and its Python dependencies  | BSD-2-Clause and their own licenses           | [streamlink/streamlink](https://github.com/streamlink/streamlink), [PyPI](https://pypi.org/)                  |
| Intel VA-API drivers, libva and gmmlib  | MIT; i965 shaders EPL-1.0 and Apache-2.0      | [iHD](https://github.com/intel/media-driver), [i965](https://github.com/intel/intel-vaapi-driver)             |
| Python 3.11 and Debian base system      | PSF-2.0 and Debian package licenses           | [python:3.11-slim-trixie](https://hub.docker.com/_/python), [Debian](https://sources.debian.org/)             |
| Rust crates compiled into the binaries  | MIT, Apache-2.0 and others                    | [`Cargo.lock`](Cargo.lock)                                                                                    |
| npm packages bundled in the web app     | MIT and others                                | [`pnpm-lock.yaml`](pnpm-lock.yaml)                                                                            |

Each FFmpeg archive identifies the FFmpeg revision it was built from in its name
(for example `n9.0.2-48-gd4070a14ce`) and the matching BtbN build tag in the lock
file's `notes_url`.

The `iHD` and `i965` drivers are Debian's `non-free` builds: their GPU kernels ship
without source.

## Web app

The web app, in the server image and the Windows app, renders styled subtitles
with [JASSUB](https://github.com/ThaUnknown/jassub) (MIT). Its WebAssembly build
and fallback font contain:

| Component                     | License           | Source                                                                                  |
| ----------------------------- | ----------------- | --------------------------------------------------------------------------------------- |
| libass                        | ISC               | [libass/libass](https://github.com/libass/libass)                                       |
| FreeType 2.14.3               | FTL               | [freetype/freetype](https://gitlab.freedesktop.org/freetype/freetype)                   |
| FriBidi                       | LGPL-2.1-or-later | [fribidi/fribidi](https://github.com/fribidi/fribidi)                                   |
| HarfBuzz                      | MIT               | [harfbuzz/harfbuzz](https://github.com/harfbuzz/harfbuzz)                               |
| Brotli                        | MIT               | [google/brotli](https://github.com/google/brotli)                                       |
| Liberation Sans, the fallback | OFL-1.1           | [liberationfonts/liberation-fonts](https://github.com/liberationfonts/liberation-fonts) |

The build also contains the Emscripten runtime and code under the other licenses
its npm package declares (MIT-Modern-Variant, NTP, Zlib and BSL-1.0); the
JASSUB repository pins every library as a submodule. Portions of this software
are copyright © 2026 The FreeType Project (https://freetype.org). All rights
reserved.

## Windows app

The installer contains the Thelxinoe desktop app, Rust crates and npm packages
listed in the lock files above, and Microsoft's WebView2 bootstrapper. The player
and its plugins (mpv, uosc, thumbfast, sub-select) are downloaded from their
publishers when you install them in **Settings → MPV**, under their own licenses.

## Source for GPL and LGPL components

For three years after a release that contains them, the complete corresponding
source of its GPL- and LGPL-licensed components is available on request: open an
[issue](https://github.com/Valtrius/thelxinoe/issues) naming the release, and it
will be provided at no more than the cost of distribution.
