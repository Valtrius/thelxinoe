# syntax=docker/dockerfile:1
FROM node:24-bookworm-slim@sha256:0e0ff40c39bc087845bfb27465a0df4ea419520094bc35842ff83dd8cbe6f9b6 AS web
WORKDIR /src
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY frontend/package.json frontend/package.json
RUN corepack enable && pnpm install --frozen-lockfile
COPY frontend frontend
COPY scripts/version.mjs scripts/version.mjs
COPY Cargo.toml Cargo.toml
COPY compose.yaml compose.test.yaml compose.release.yaml Cargo.lock Dockerfile ./
COPY apps/desktop/tauri.conf.json apps/desktop/tauri.conf.json
RUN pnpm run build:web

FROM rust:1.98-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY apps apps
# COPY timestamps can predate a different source build in the shared cache.
# Refresh local inputs and hold the cache until both binaries are copied out.
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target,sharing=locked find apps crates -type f -exec touch {} + && cargo build --locked --release -p thelxinoe-server -p thelxinoe-docker-controller && cp target/release/thelxinoe-server target/release/thelxinoe-docker-controller /usr/local/bin/

FROM python:3.11-slim-trixie@sha256:da047cb8f9d1d98e5c070f5300ba9f7274e33b8fc0e5be5ed88740aed1b95ba9 AS tools
WORKDIR /src
COPY apps/server/src/tools/package.py apps/server/src/tools/package.py
COPY apps/server/src/online/youtube_worker.py apps/server/src/online/streamlink_worker.py apps/server/src/online/
COPY scripts/prepare-server-tools.py scripts/server-tools.lock.json scripts/
RUN python scripts/prepare-server-tools.py /opt/thelxinoe/tools --compact

FROM python:3.11-slim-trixie@sha256:da047cb8f9d1d98e5c070f5300ba9f7274e33b8fc0e5be5ed88740aed1b95ba9 AS server
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl tini && rm -rf /var/lib/apt/lists/* && groupadd -g 10001 thelxinoe && useradd -u 10001 -g 10001 thelxinoe && mkdir -p /var/lib/thelxinoe /var/cache/thelxinoe /run/thelxinoe && chown -R 10001:10001 /var/lib/thelxinoe /var/cache/thelxinoe /run/thelxinoe && chmod 2770 /run/thelxinoe
ARG VERSION=0.1.0
ARG REVISION=development
LABEL org.opencontainers.image.source="https://github.com/Valtrius/thelxinoe" org.opencontainers.image.version=$VERSION org.opencontainers.image.revision=$REVISION
COPY releases/release.pub /etc/thelxinoe/release.pub
COPY --from=rust /usr/local/bin/thelxinoe-server /usr/local/bin/
COPY --from=tools /opt/thelxinoe/tools /opt/thelxinoe/tools
COPY --from=web /src/frontend/dist /opt/thelxinoe/web
ENV THELXINOE_BIND=0.0.0.0:8484 THELXINOE_STATE=/var/lib/thelxinoe THELXINOE_CACHE=/var/cache/thelxinoe THELXINOE_WEB=/opt/thelxinoe/web THELXINOE_MEDIA=/media
USER 10001:10001
EXPOSE 8484
HEALTHCHECK --interval=15s --timeout=3s CMD curl -fsS http://127.0.0.1:8484/api/v1/health || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/thelxinoe-server"]

FROM debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251 AS controller
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl tini && rm -rf /var/lib/apt/lists/* && mkdir -p /run/thelxinoe && chown 0:10001 /run/thelxinoe && chmod 2770 /run/thelxinoe
ARG VERSION=0.1.0
ARG REVISION=development
LABEL org.opencontainers.image.source="https://github.com/Valtrius/thelxinoe" org.opencontainers.image.version=$VERSION org.opencontainers.image.revision=$REVISION
COPY releases/release.pub /etc/thelxinoe/release.pub
COPY --from=rust /usr/local/bin/thelxinoe-docker-controller /usr/local/bin/
USER 0:10001
HEALTHCHECK --interval=15s --timeout=3s CMD curl -fsS --unix-socket /run/thelxinoe/controller.sock http://localhost/health || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/thelxinoe-docker-controller"]
