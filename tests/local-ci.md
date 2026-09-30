# Local CI execution contract

Run the same phase commands as GitHub against a snapshot that includes pending
working-tree changes. Qualify the runner through the complete local CI run and
retain its lane timestamps, logs, summary, source identity and test artifacts.

Failures to cover before implementation:

- Dependency installation unlinks a native module used by the development app.
- Concurrent frontend builds, Rust outputs, installers, fixtures or test reports
  overwrite another lane's files or silently qualify a different source snapshot.
- A second local run races fixed browser ports or Docker fixture names, or its
  cleanup removes the first run's containers.
- The aggregate run omits a remote phase, uses the combined update scenario in
  place of the separate remote lanes, or stops observing lanes after one fails.
- Installation or process launch failure is mistaken for a successful test run.
- Interleaved logs hide the failing lane, pending results appear successful, or
  the completion window belongs to an unidentified run.
- Local modifications or deleted files are omitted from isolated workspaces.
- A report says it refreshes every five seconds but publishes stale data, or
  opens before it exists. Failure to open the browser must not skip test lanes.
- A hidden coordinator hides its report too, or activates the user's browser.
  Request background opening through the default browser, retain launch errors,
  and never bring an existing window forward.
- The terminal session ends during a long container lane, killing the coordinator
  and leaving its report running. Launch the local coordinator in an independent
  hidden process; preserve an interrupted result if it exits without finishing.
- Detached Windows PowerShell exits successfully without executing its report
  helper. Keep that helper hidden but attached, and retain its shell acknowledgement.
- Parallel Docker builds share a Cargo output directory after Cargo releases
  its lock, allowing another build to change binaries before they are copied.
- Concurrent database openers publish the schema but race switching to WAL.
  Busy/locked journal transitions must have a bounded wait; incompatible
  databases and other SQLite errors must still be rejected. The existing
  concurrent initialization test covers openers the E2E server lock excludes.
- A fixture submits setup while its proxy is running but TLS is not ready.
  Wait using a read-only public health request; never replay an ambiguous POST.
- Desktop browser fixtures save navigation under the web server key and never
  reach the settings surface whose update response-loss behavior is being tested.

Container runtime reductions retain real file watcher add/remove assertions in
a Linux-owned fixture volume. They must fail within fifteen seconds if watching
breaks, rather than pass through the five-minute reconciliation fallback.
Track-selection checks consume the existing playback catalog without rescanning
unchanged files; the catalog scenario already checks explicit rescanning.
Build production server/controller images once and tag those same binaries for
service fixtures. Service access and service connections keep separate fresh
deployments because they test distinct authorization and lifecycle behavior.

Each lane owns its worktree, dependency installation, build outputs and evidence.
Only one local coordinator may hold the machine's fixed test fixtures at a time;
all lanes within that coordinator run concurrently. Dependency installation and
shared browser provisioning are preparation, not passing test evidence.
