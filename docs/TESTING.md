# Test commands

Run from the repository root after `pnpm install --frozen-lockfile`. Install Rust 1.96+, Node 24+, pnpm 12.18.0, FFmpeg/FFprobe, Docker (Linux containers) and Playwright Chromium (`pnpm exec playwright install chromium`).

Prefer complete application workflows; extend an existing scenario when possible.

```sh
pnpm run validate          # Formatting, lint, Rust/web checks, unit tests and web build
pnpm run test:ui           # Browser checks against routed fixtures
pnpm run test:ui:mock      # Browser checks against the mock preview
pnpm run ci <phase...>     # CI phases, for example `services media`
```

[`scripts/ci.mjs`](../scripts/ci.mjs) defines the phases and GitHub Actions runs each
as its own job. Container phases build this revision's images when they are missing.
On Windows, Linux checks require Docker Desktop running Linux containers.

Playwright saves an HTML report under `playwright-report/` and traces of failed
tests under `test-results/`. CI uploads both when a job fails. Container fixtures
save their results, screenshots and service logs under `.local/`. Traces of the
media services' own interfaces are omitted because their responses contain API keys.

No other container may mount the transfer fixture folders: approval then needs a
declaration the transfer tests don't make, so stop development containers that
bind-mount the worktree before `pnpm run ci transfers`.

Use the [update lab](UPDATES.md) for server and desktop app updates. Protocol tests
alone do not establish client compatibility: repeat login, browsing, playback and
discovery in actual clients when changing their protocols.

## Why isolated coverage remains

These are existing exceptions, not permission to add tests after implementation.
Before adding an exception, enumerate its failure modes and establish why the
application workflows cannot reliably exercise them.

| Area                                         | Failure missed by the current application workflows                                                                                                                                                    |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Database, jobs, event writes                 | Concurrent readers/writers, queue overload, panic or event-write failure must preserve transaction boundaries and accepted work.                                                                       |
| Authentication and provider sessions         | Cross-user credentials, forged transports, quota races, and late refresh/authorization replies must not bypass revocation or restore deleted data. Live-provider smoke tests cannot force these races. |
| Owned media services and auto-delete         | Lost upstream replies, ambiguous Docker state, foreign ownership, default quality filtering, changed file versions, and exact deletion eligibility need controlled fault injection.                    |
| Archives, releases, paths, subprocesses      | Malformed packages, traversal, symlinks, corrupt signatures, oversized/truncated output, and child-process escapes are absent from normal fixture inputs.                                              |
| Playback, catalog, skip segments, statistics | Conditional client capabilities, replacement/eviction, duplicate music recordings across albums, false intro matches, suspension, and DST boundaries are absent from the ordinary media fixtures.      |
| Desktop tool selection and updates           | Concurrent saves/downloads, revoked selections, release replacement, rate-limit recovery, leases, and cleanup are not exercised by the MPV player smoke tests.                                         |
| MPV configuration                            | Profiles, negated options, duplicate assignments, unsaved drafts, and imported plugins can corrupt saved configuration; the browser suites do not run the desktop app's configuration editor.          |
| Web app transport and defensive inputs       | Reconnect cursor ordering, failed discovery retry, malformed provider links/progress, and unsafe service URLs are not supplied by the existing browser fixtures.                                       |

Constant tables, field-copy assertions, fake-DOM geometry, mock call sequences,
source-layout checks, and happy paths already exercised through the application
do not justify another test.
