# Thelxinoe documentation

Start with [product scope](PRODUCT.md), [implementation status](STATUS.md) and [local verification](TESTING.md).

## Design and operations

| Topic                                            | Document                                  |
| ------------------------------------------------ | ----------------------------------------- |
| Runtime boundaries, code layout and jobs         | [Architecture](ARCHITECTURE.md)           |
| Persistent paths and shared media mounts         | [Storage](STORAGE.md)                     |
| Accounts, secrets and transport separation       | [Security](SECURITY.md)                   |
| Theme, navigation and platform-specific settings | [Appearance](APPEARANCE.md)               |
| Catalog identity, metadata, playback and music   | [Media model](MEDIA-MODEL.md)             |
| HTTPS, proxy trust, CORS and PWA                 | [Remote access](REMOTE.md)                |
| YouTube, Twitch and Kick                         | [Online providers](ONLINE.md)             |
| Wholphin and Jellyfin Android TV                 | [Client compatibility](JELLYFIN.md)       |
| Acquisition and manager ownership                | [Acquisition](ACQUISITION.md)             |
| Container installation and adoption              | [Managed stack](MANAGED-STACK.md)         |
| Service update isolation and recovery            | [Service updates](SERVICE-UPDATES.md)     |
| Watched media cleanup                            | [Retention](RETENTION.md)                 |
| Intro, recap, credits and preview skipping       | [Segments](SEGMENTS.md)                   |
| Health, users, diagnostics and backups           | [Operations](OPERATIONS.md)               |
| Signed product releases and offline recovery     | [Releases](RELEASES.md)                   |
| Required release checks                          | [Release checklist](RELEASE_CHECKLIST.md) |

## Accepted delivery criteria

The original planning documents are consolidated here. Each implementation phase must provide a working end-to-end path and test its real protocol or provider contract where applicable. Mock tests complement live validation. The criteria below preserve the original delivery requirements; STATUS records actual completion and limits.

- [Foundation and identity](acceptance/foundation.md)
- [Local media and clients](acceptance/local-media.md)
- [Online providers](acceptance/online-media.md)
- [Managed services and retention](acceptance/managed-media.md)
- [Operations and release](acceptance/operations-release.md)

Features deliberately deferred beyond v1 are listed in [product scope](PRODUCT.md#v1-exclusions).
