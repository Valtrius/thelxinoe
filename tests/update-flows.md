# Update qualification

`npm run updates:lab -- start` leaves an isolated installation running for manual
review. `npm run test:updates` uses the same publisher, keys and builds, then
removes its containers and uninstalls its Windows app. See [commands](../docs/UPDATES.md).

The automated scenarios must catch these failures through actual processes:

- A fresh GitHub runner has no registry/proxy images, and Docker Hub resets its
  token connection during an implicit `docker run` pull. Explicitly fetch both
  fixture images with bounded retries and command timeouts before creating the
  lab's containers; an exhausted pull must still fail qualification.
- Signed feed redirects fail, signatures/expiry are ignored, unavailable feeds
  are reported as current, or a fresh current server hides publisher metadata.
- A new server release never reaches an administrator's inbox, a normal user
  gets update controls, or restored event IDs leave an open client stale.
- Preparation damages running state; activation loses settings/credentials;
  server and controller run different versions; accepted Compose pins are lost.
- One click requires another confirmation, queues a different release, starts
  during playback, duplicates an operation on retry, or loses its intent when
  Settings closes or the server restarts between preparation and activation.
- The version icon reports success before acceptance, hides a failed check,
  treats a disconnect as an install failure, or reinstalls/restores state when
  the user only asks to check the connection. Keyboard and reduced-motion users
  must be able to inspect the same state without a shifting version label.
- A controller handoff bypasses the two-minute reconnect grace period, pointer
  clicks leave tooltips stuck open, or download rings invent progress instead
  of using Docker layer bytes / signed installer bytes.
- The reconnect test advances its clock before the controller failure reaches
  the browser, so the grace period begins after the jump on a slower GitHub
  runner. Install the clock before navigation starts polling, hold the failure
  response to verify the preceding state, and observe reconnect before advancing
  time. Retain the browser trace, clock observations and reconnect screenshot.
  Service onboarding's polling and feedback timers must likewise start with the
  installed clock; installing it after navigation leaves native timers running
  while the test advances or pauses a different clock. Pause a fixed clock on
  the blank page before navigation; pausing at the host's current time can race
  ahead of the browser and attempt to move its clock backwards.
- A desktop check starts downloading without a second click, or updater exit
  loses the current window position, size or maximized state.
- Cached lab layers hide server download progress; slow-download must exercise
  actual registry transfers before installation. Show a partial percentage only
  when Docker supplies total bytes, otherwise keep the orbit indeterminate.
  Completing a layer with an unknown size must preserve its observed byte count.
  Refresh registry-only layers after an earlier upgrade; deleting image references
  alone does not guarantee that Docker's content store will transfer bytes again.
- Lab stop leaves its Windows uninstall entry behind, fails when repeated, or
  removes another lab's installation or the normal desktop app.
- A lab installer kills another desktop by executable name, or concurrent lab
  builds package each other's binary or installer. Keep an unrelated process
  with the production executable name alive through installation and cleanup.
- Update status changes move the settings below the version card, or expected
  maintenance responses insert page-wide errors during preparation/reconnect.
- Desktop one-click updates lose their intent after navigation, restart during
  playback, install a changed or corrupt candidate, or run automatic installs
  before sign-in. The login page must expose updates only for incompatibility.
- Sign-in sends credentials to the previous server, skips the newly entered
  server's compatibility/setup check, or requires a separate connection click.
- Web clients require a second reload click, retain newer assets after rollback,
  or reload during playback after an accepted server update.
- Candidate validation fails after modifying copied/live state, or the controller
  is interrupted, and the original state cannot recover with cached images.
- A recovered server's startup scan is still running when the next qualification
  asks for preflight or activation. Wait only after the explicit idle-gate 409;
  fail immediately on transport ambiguity or any other rejection, and retain the
  observed jobs so retries cannot hide unfinished work.
- A desktop needs its server to discover updates, downloads the wrong bytes,
  accepts corrupt signatures/metadata, restarts during playback, or loses its
  login/configuration across the real Windows installer and relaunch.
- Obsolete preflight recovery poisons the journal or controller restart, or a
  ready preflight invalidated by Compose recreation stalls Automatic. Recreate
  before activation, qualify a new generation, then commit and restore it.
- Server restart during image pulls admits playback or scan jobs before its
  snapshot. A lost controller acknowledgement must keep maintenance until a
  complete, settled controller observation establishes rejection or readiness.
- Installation races an accepted desktop play request still resolving tools,
  or terminates an active package operation. Hold those actual command boundaries
  in disposable builds and reject installation only after both have drained.
- Automatic leaves a retained verified installer unused after compatibility
  recovers. Corrupt the publisher's download after retention so a successful
  installation also proves that it reused the verified bytes.
- Restoring the base desktop preserves its session but reconnects before the UI
  finishes booting. Wait for the authenticated controls or the sign-in form before
  choosing whether to sign out; otherwise the test fills Settings' server field
  and waits for a login form that never appears.
- The fixture finishes its slow download before progress assertions run, so a
  successful installer closes the page being inspected. Hold the real HTTP stream
  after its first chunk until progress evidence is captured, and always release it.
- Failure evidence follows the previous desktop connection after a relaunch.
  Keep the current connection available to the scenario runner's cleanup.
- A desktop retains a lost install-response error after authoritative pending
  and committed polls. Verify its final success presentation without a reload.

Server tests use disposable Docker deployments and the production HTTP/UI paths.
Windows tests install an independently named app with its own keyring identity.
Only fixture source copies contain validation failure/hold points; no test bypass
or schema migration is added to production. Linux-only server runs use an unused
desktop placeholder; the separate Windows lane qualifies actual NSIS artifacts.

Keep results, screenshots, sanitized operation states and browser traces under
`test-results/updates`. The fixtures contain only generated test accounts.
Assertions verify outcomes and recovery, not timing, pixel layout or implementation
structure. Manual review covers wording, transitions and visual details.
