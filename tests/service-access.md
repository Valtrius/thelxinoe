# Native service access E2E contract

Run `npm run test:service-access` against fresh, isolated storage. The runner
builds the current server/controller and installs the digest-pinned services.
No previous Thelxinoe installation is upgraded or repaired.

Failures to cover before implementation:

- An attached service with a usable existing prefix cannot open its native UI
  or use a desktop ticket, or connecting it changes its existing configuration.
- Empty, application-reserved or overlapping prefixes publish unsafe native routes
  or prevent an otherwise valid API connection. NZBGet needs no private prefix.
- Ownership transfer fails to set the fixed prefix before starting the copy,
  changes unrelated settings, or modifies the original appdata.
- A blocked transfer restores the original container with the replacement's
  prefix; retries, recreation or enabled service links retain a stale prefix.
- Managed URL Base remains editable through Thelxinoe, or removing the editor
  also removes the existing-prefix input needed to connect external APIs.
- NZBGet's root-based assets, JSON-RPC, settings and uploads fail through its
  public mount, or its private API is incorrectly given that mount as a URL Base.
- NZBGet asks for a second Basic login, forwards browser-supplied credentials,
  exposes an upstream authentication challenge, or allows cross-origin RPC calls.
- NZBGet native access bypasses administrator checks, device-grant
  scope or revocation.
- Lidarr, Prowlarr or Bazarr native assets, deep links, settings saves and live
  events escape their prefix; Bazarr's saved base uses a trailing slash.
- Bazarr interprets a generated Docker hostname as a number and rejects native
  settings saves; fresh managed and attached fixtures need a string hostname.
- Support-service access bypasses authorization or loses URL Base during
  recreation.
- Prowlarr callbacks omit its own prefix, Bazarr settings writes use its old root,
  or update preflight cannot discover Bazarr's configured base.
- Prowlarr rejects the public browser hostname despite accepting the registered
  private API address; attached services must not need host-allowlist mutations.
- Adoption retains a restrictive Prowlarr allowlist that rejects its new stable
  container name; the copied allowlist must preserve existing entries and allow it.
- Private Radarr/Sonarr cannot load behind the one public HTTPS origin; assets,
  deep links, refresh, native settings, SignalR or downloads use the wrong base.
- URL Base breaks Thelxinoe API calls, artwork, enabled Prowlarr/Bazarr/Seerr
  connections, controller preflights or container replacement.
- The private root address stops redirecting to the configured base.
- Anonymous users, regular users, native API keys or forged forwarding headers
  bypass Thelxinoe authentication. Cross-origin forms or WebSockets mutate state.
- Traversal, ambiguous paths, redirects or conflicting bases reach another
  service, Thelxinoe endpoint or arbitrary destination.
- Thelxinoe credentials reach the service; native cookies collide with application
  cookies or escape their service path; a service worker controls the application.
- Desktop launch leaks the device token, reuses a launch ticket, grants access to
  another service or keeps access after logout, expiry, revocation or role change.
- Streams or requests awaiting response headers remain open after access is
  revoked or the registered container changes.
- Attached services cannot use their existing API prefix, including a prefix
  that overlaps an application route; only unambiguous native routes are exposed.
- Native configuration drift yields a broken page or silently overwrites settings.
- Fresh External-authentication settings fail Servarr's General-settings validator,
  preventing an administrator from saving native settings.
- Stop/start, recreation, preflight/activation and recovery lose routing settings.
- Prowlarr's host allowlist retains a previous container IP after restart and
  rejects API calls or Radarr/Sonarr callbacks using the newly assigned IP.

Evidence: `test-results/service-access/result.json`, pinned image identities and
screenshots of native pages. Do not record raw browser traces: native UI responses
contain service API keys. The runner records safe assertion names and statuses.
Existing connection and update suites remain responsible for their full lifecycle
contracts; extend them to run with prefixed services rather than duplicate them.

Real Servarr cannot deliberately emit hostile cookies/redirects or an endless
test stream. `service-access-peer.mjs` fills that specific security coverage gap
through the running server and controller; normal UI/API checks use real services.
Service attachment and adoption are covered through the public API.
The adoption recovery scenario injects a database write failure after API
registration to verify restoration of the original container and prefix.
