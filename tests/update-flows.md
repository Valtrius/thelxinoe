# Update qualification

`npm run updates:lab -- start` leaves an isolated installation running for manual
review. `npm run test:updates` uses the same publisher, keys and builds, then
removes its containers. See [commands](../docs/UPDATES.md).

The automated scenarios must catch these failures through actual processes:

- Signed feed redirects fail, signatures/expiry are ignored, unavailable feeds
  are reported as current, or a fresh current server hides publisher metadata.
- A new server release never reaches an administrator's inbox, a normal user
  gets update controls, or restored event IDs leave an open client stale.
- Preparation damages running state; activation loses settings/credentials;
  server and controller run different versions; accepted Compose pins are lost.
- Candidate validation fails after modifying copied/live state, or the controller
  is interrupted, and the original state cannot recover with cached images.
- A desktop needs its server to discover updates, downloads the wrong bytes,
  accepts corrupt signatures/metadata, installs without the restart action, or loses its
  login/configuration across the real Windows installer and relaunch.

Server tests use disposable Docker deployments and the production HTTP/UI paths.
Windows tests install an independently named app with its own keyring identity.
Only fixture source copies contain validation failure/hold points; no test bypass
or schema migration is added to production. Linux-only server runs use an unused
desktop placeholder; the separate Windows lane qualifies actual NSIS artifacts.

Keep results, screenshots, sanitized operation states and browser traces under
`test-results/updates`. The fixtures contain only generated test accounts.
Assertions verify outcomes and recovery, not timing, UI geometry or implementation
structure. Manual review covers wording, transitions and visual details.
