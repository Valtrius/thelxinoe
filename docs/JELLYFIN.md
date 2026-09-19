# Jellyfin client compatibility

The adapter is under development (roadmap phase 8). It uses Thelxinoe's catalog, authentication sessions, playback grants, progress, and history. It does not run a Jellyfin server or expose Jellyfin administration APIs.

## Protocol findings

- Wholphin 1.0.8 uses Jellyfin Kotlin SDK 1.7.1. Discovery requires `ProductName: "Jellyfin Server"`. The adapter keeps the actual product and release in `ServerName` and `ThelxinoeVersion`; `Version: "10.10.7"` identifies the compatibility contract. Jellyfin Android TV 0.19.10 currently warns that its next release will require 10.11; that newer contract still needs validation.
- Kotlin model constructors require more fields than the OpenAPI `required` lists suggest. Authentication includes the required user policy, configuration, and session fields. Playback sources and streams include the required booleans and protocol enums.
- Clients repeat list query parameters, including fields and image types. The adapter merges documented lists and rejects conflicting scalar parameters, including credentials and user identity.
- API requests use a revocable compatibility device token. Header and query credential transports are restricted to compatibility routes. They cannot authenticate first-party APIs, and first-party tokens cannot authenticate the compatibility API.
- Wholphin's gallery sends the device authorization header, but its video player uses an unauthenticated media data source. It constructs `/Videos/{id}/stream` and preserves the source `ETag` as `tag`. Playback-info therefore places a short-lived, session-bound playback grant in that field. The direct URL returned by the adapter also uses the existing granted playback endpoint. Stopping or revoking the parent device invalidates these URLs.
- Artwork tags carry short-lived artwork grants so the client's player artwork request can authenticate too. Authenticated gallery requests remain supported.
- Wholphin requests cinema intros before starting playback. The adapter returns an empty intro list because no cinema intro source is configured.
- Quick Connect stores hashes of its secret and six-digit code, expires after five minutes, and requires a signed-in user to inspect and approve the named device. Approval is tied to that user's session with a separate, single-use confirmation grant. The client exchanges the approved secret once. Polling must retain the same display code; Jellyfin Android TV redraws it from every poll result.
- Compatibility debug logs contain only an allowlisted route shape, method, and status. They omit arbitrary path components, all query values, headers, and bodies.

## Validation in progress

The local test server uses Docker project `thelxinoe-compat`, HTTP port 18787, HTTPS port 21443, and a dedicated state volume. The Android TV API 34 emulator is `emulator-5580`; its host address is `10.0.2.2:18787`.

| Check                                  | Wholphin 1.0.8                                 | Jellyfin Android TV 0.19.10 |
| -------------------------------------- | ---------------------------------------------- | --------------------------- |
| Manual server connection               | Passed                                         | Passed                      |
| Password login                         | Passed                                         | Pending                     |
| Quick Connect through Thelxinoe web UI | Pending                                        | Passed                      |
| Library navigation and movie list      | Passed                                         | Passed                      |
| Real cached TMDB artwork               | Pending                                        | Passed                      |
| Direct H.264/AAC playback              | Passed                                         | Pending                     |
| Progress, completion, history          | Passed: 18-second clip, 17.901 seconds counted | Pending                     |
| Subtitle display and selection         | Pending                                        | Pending                     |
| Remux, transcode, seek and resume      | Pending                                        | Pending                     |
| Music, shared playlists, favorites     | Pending                                        | Pending                     |
| UDP discovery                          | Pending                                        | Pending                     |

`scripts/test-jellyfin.mjs` validates authentication, browsing, search, token transport separation, granted direct byte ranges, progress and watched translation, stopped-stream rejection, and absence of token/grant values from Docker logs. It uses synthetic playback fixtures. `scripts/prepare-tv-metadata.mjs` explicitly uses the private local provider files to attach real TMDB artwork to that synthetic fixture. `scripts/test-tv-quick-connect.mjs` approves the code currently displayed in the emulator through the real web interface.

Rust integration tests cover device revocation, cross-user rejection, credential conflicts, and the Quick Connect approval and single-use boundaries. All previous server integration tests still pass.

Phase 8 remains open until the remaining client checks and required capabilities pass. In particular, converted playback currently uses the first-party rolling HLS pipeline. Unmodified TV clients require a complete seekable VOD timeline, which is the next playback work item.
