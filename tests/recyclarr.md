# Recyclarr Docker E2E contract

Run `npm run test:recyclarr` after building the service server/controller images.
Results are recorded in `.local/recyclarr-result.json` and
`.local/recyclarr-qualification-result.json`; each report identifies its retained
snapshots, redacted output, and browser trace directory.

Failure cases to cover before implementation:

- The Linux runner owns the fixture directories with a UID other than 1000.
  Run the qualification CLI and Arr fixtures with the invoking process's UID/GID
  so bind mounts remain writable and their generated keys can be removed.
- A command exits successfully but an idle installation is reported unhealthy.
- Installation or recreation starts the job definition's `--version` command.
  Immediate preflight can observe it running and attempt an HTTP activity check
  against a nonexistent integration. Verify the definition stays Created before
  the first update, and after recreation, then qualify/apply with preserved state.
- A deliberately running definition reaches an HTTP activity probe, or cannot
  be stopped without API credentials. Pause its real process to retain the busy
  state, verify preflight blocks with a job-specific idle error, then stop it.
- A job accidentally exposes a port, mounts media, starts cron, or writes as root.
- Docker reports no published ports as either `null` or an empty object.
  Verify zero binding keys instead of requiring one JSON representation.
- Catalog labels change or a selected TRaSH ID disappears.
- Catalog loading is pending or rejected when a GitHub runner's browser
  dropdown-count deadline expires. Observe both real catalog HTTP responses
  before asserting the choices, fail on rejected responses, and retain response
  status, a failure screenshot, and the browser trace in GitHub artifacts.
- Preview writes Arr settings; repeated application duplicates profiles or CFs.
- Arr answers HTTP before initializing its built-in profiles; an empty baseline
  then mistakes native startup writes for preview writes. Wait for populated,
  stable native settings before recording the fresh qualification snapshot.
- CF definitions, scores, and quality settings disagree with the selected guide.
- Initial sync or profile switching fails and destroys the last valid default.
- New requests use a stale default; existing movies or series are reassigned.
- Seerr fails to receive the current numeric profile, label, or UHD classification.
- Manual, daily, retry, update, removal, and backup operations race over state.
- Restart duplicates an overdue sync or reuses a stale target generation.
- Partial API writes are reported as successful; retries lose tracked identities.
- CLI errors return zero and incorrectly advance the acquisition defaults.
- An unavailable or unauthorized target retries forever or silently changes guide.
- An image candidate reaches production Arr during preflight.
- Restore or adoption loses state, credentials, ownership, or enables two schedulers.
- Secrets appear in Docker environment/specs, queued payloads, public history,
  browser controls, exports, logs, or retained evidence.
- A queued actor loses administrator permission but the operation still applies.
- Radarr/Sonarr profile controls remain available and compete with Recyclarr.
- Installing Radarr/Sonarr before Recyclarr creates a competing custom profile.
  The service-connections fixture checks fresh installs and repeated options
  reads against the real Arr profiles; initial defaults reuse a built-in profile.
- A Docker command or HTTP request never returns, leaving the test unbounded.
- Browser trace shutdown fails and skips container cleanup or the result file.
  `node scripts/test-recyclarr-cleanup.mjs` interrupts the real fixture after
  installation and closes its browser, then verifies a failed, finished report
  and no containers belonging to its deployment/project.

Use real containers and authenticated HTTP/UI workflows. No new isolated tests
are planned; any necessary coverage gap must be documented here first.
