# syntax=docker/dockerfile:1
FROM node:24-bookworm-slim AS web
WORKDIR /src
COPY package.json package-lock.json ./
COPY frontend/package.json frontend/package.json
RUN npm ci
COPY frontend frontend
COPY scripts/version.mjs scripts/version.mjs
COPY Cargo.toml Cargo.toml
COPY compose.yaml compose.test.yaml ./
COPY apps/desktop/tauri.conf.json apps/desktop/tauri.conf.json
RUN npm run build

FROM rust:1.98-bookworm AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY apps apps
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target cargo build --locked --release -p thelxinoe-server -p thelxinoe-docker-controller && cp target/release/thelxinoe-server target/release/thelxinoe-docker-controller /usr/local/bin/

FROM python:3.11-slim-bookworm@sha256:a36c24f9cbdf4fd0f52d67f0823eeac19c2028c637cecc392d97f980d4fec56b AS streamlink
COPY scripts/streamlink-requirements.txt /requirements.txt
RUN python -m venv /opt/streamlink && /opt/streamlink/bin/pip install --no-cache-dir --require-hashes -r /requirements.txt

FROM python:3.11-slim-bookworm@sha256:a36c24f9cbdf4fd0f52d67f0823eeac19c2028c637cecc392d97f980d4fec56b AS server
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg curl tini && rm -rf /var/lib/apt/lists/* && groupadd -g 10001 thelxinoe && useradd -u 10001 -g 10001 thelxinoe && mkdir -p /var/lib/thelxinoe /var/cache/thelxinoe /run/thelxinoe && chown -R 10001:10001 /var/lib/thelxinoe /var/cache/thelxinoe /run/thelxinoe && chmod 2770 /run/thelxinoe
COPY --from=rust /usr/local/bin/thelxinoe-server /usr/local/bin/
COPY --from=streamlink /opt/streamlink /opt/streamlink
COPY --from=web /src/frontend/dist /opt/thelxinoe/web
ENV THELXINOE_BIND=0.0.0.0:8484 THELXINOE_STATE=/var/lib/thelxinoe THELXINOE_CACHE=/var/cache/thelxinoe THELXINOE_WEB=/opt/thelxinoe/web THELXINOE_MEDIA=/data
USER 10001:10001
EXPOSE 8484
HEALTHCHECK --interval=15s --timeout=3s CMD curl -fsS http://127.0.0.1:8484/api/v1/health || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/thelxinoe-server"]

FROM debian:bookworm-slim AS controller
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl tini && rm -rf /var/lib/apt/lists/* && mkdir -p /run/thelxinoe && chown 0:10001 /run/thelxinoe && chmod 2770 /run/thelxinoe
COPY --from=rust /usr/local/bin/thelxinoe-docker-controller /usr/local/bin/
USER 0:10001
HEALTHCHECK --interval=15s --timeout=3s CMD curl -fsS --unix-socket /run/thelxinoe/controller.sock http://localhost/health || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/thelxinoe-docker-controller"]
