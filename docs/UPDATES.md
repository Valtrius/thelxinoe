# Updates

Server and Windows desktop share one product version. GitHub Releases supplies
the signed publisher envelope and Windows installer; GHCR supplies Linux amd64
server/controller images pinned by digest. Web assets travel with the server.

Server **Notify** discovers releases and adds an administrator notification that
opens Settings → Server. **Automatic** also prepares and installs during the
configured idle maintenance window; equal start and end hours keep it open all
day. The server checks hourly, after each start and when the window opens.
Preparation tests copied state and recovery; installation replaces both
containers. Failed activation restores the snapshot. Automatic retries a release
blocked by Docker, disk or downloads after a growing pause, five times at most,
and never retries one that failed validation or cannot migrate this server.
Click the icon beside **Product version** once to queue the full server update.
It downloads both images while the server keeps working, waits for playback and
background work, prepares, installs and reconnects. The server is unavailable
only while it is copied and replaced. Closing Settings, restarting the server or
an unavailable Docker does not cancel it, and a newer release does not replace
it; **Cancel** does until installation starts. Hover or focus the icon for its
current step. Idle web tabs reload automatically after acceptance. Failed
attempts can be dismissed; attempts for containers since recreated, for example by
`docker compose up`, no longer count. Update preferences sit below the version
cards. **Rollback** on the version card opens snapshot recovery.
Restoring a snapshot also resets the event connection and refreshes client state.

Desktop checks the publisher directly, including before login or while the server
is unavailable. In Settings → Desktop, and on the sign-in and update-required
screens, the quiet version icon checks for updates; the available-update arrow
downloads and installs. **Automatic** does this after login, or as soon as the
server reports this desktop as too old. Restart waits for playback to stop. A
release the reachable server cannot accept waits, saying to update the server
first, and installs once the server accepts it; an unreachable server never holds
an update. Downloads are held for the current desktop session, also when starting
the installer fails, and Automatic retries a failed start after a growing pause.
See [server policy](../apps/server/src/product.rs),
[controller recovery](../apps/docker-controller/src/product.rs) and
[desktop updater](../apps/desktop/src/updates.rs).

## Manual local lab

Install the normal build prerequisites, Docker with Linux containers, Node 24,
Rust, and OpenSSL (Git for Windows supplies it). Run `pnpm install --frozen-lockfile` first. Windows builds
also need the Tauri prerequisites and WebView2. The first run builds two complete
versions and can take several minutes.

```sh
pnpm run updates:lab start
# Docker/web only (also works on Linux):
pnpm run updates:lab start --server-only
# Windows installer only, with a local native API server and no Docker:
pnpm run updates:lab start --desktop-only
```

The command prints the publisher URL, server URL and `lab.json` path. The Windows
lab opens a separately named desktop with its own executable, installation,
keyring identity and WebView profile. Concurrent lab builds wait for each other's
Windows packaging step. It starts at the repository version and offers the next patch
version. Test keys, source copies and artifacts stay under `.local/<lab-id>`.
Production sources, trust keys and product version are not changed by the lab.

1. Open the printed server URL and create `admin` with password
   `update lab passphrase`. Connect the lab desktop to the same URL and sign in.
2. Open the printed HTTPS publisher URL. Accept the browser's warning for this
   disposable local certificate; the lab apps trust its generated CA explicitly.
   Select **candidate** and apply. Alternatively:
   `pnpm run updates:lab candidate "<lab.json>"`.
3. In either web or desktop, open Settings → Server. The quiet icon beside
   **Product version** checks for a release; once available it becomes an animated
   arrow. The notification's **View server version** action opens this screen.
   Click the arrow once and keep both clients open to review progress and reconnect.
   Repeat after a lab reset from the other client. Start playback before clicking
   to review the queued state, then stop playback and let the update continue.
4. In Settings → Desktop, use the quiet icon beside **Desktop version** to check,
   then click the available-update arrow. Check the version, retained login and
   window position after the installer relaunches it.
5. Publisher choices also exercise unavailable/expired/forged feeds and mismatched
   metadata/corrupted installers. Use **Check** after changing the publisher.
   Choose **slow-download** to inspect server and desktop download progress.
   Reset regenerates uncached server layers so the next run downloads again.
   Review playback gating with actual playback in the lab if relevant to UI work.

To review server recovery, run
`pnpm run updates:lab fail-validation "<lab.json>"` before clicking the update icon.
The candidate deliberately fails validation and the controller restores the
snapshot. Run `pnpm run updates:lab clear-failure "<lab.json>"` before retrying.

```sh
pnpm run updates:lab reset "<lab.json>"
pnpm run updates:lab stop "<lab.json>"
```

Reset recreates the Docker baseline and reinstalls the base desktop. Complete
server setup again after a Docker reset. In desktop-only mode it keeps the native
server data. Stop uninstalls that lab's Windows app, terminates lab processes and
removes its Docker containers and volumes; bind-mounted state, build artifacts
and the isolated Windows profile are kept. Each Docker reset uses a new state directory.
The lab publisher listens on the host so Docker can reach it; its control endpoint
accepts loopback requests only. No private signing key is mounted in containers.

## Automated qualification

```sh
pnpm run test:updates                 # Windows: Docker + native desktop
pnpm run test:updates --server-only # Linux or Windows
pnpm run test:updates --desktop-only
```

Each run saves `test-results/updates/<timestamp>/result.json`, screenshots,
Playwright traces and operation/process logs. Open a trace with
`pnpm exec playwright show-trace <trace.zip>`. `--keep` retains the lab for inspection;
`--lab "<lab.json>"` uses an existing lab after a reset. Generated credentials are
test-only. Failure points are injected solely into disposable candidate sources.
No production migrations or test switches are added.

The regular CI includes separate Linux server and Windows desktop lanes. Local
`pnpm run ci` runs both qualification lanes separately on Windows, using the same
entry points as GitHub: `ci:updates:server` and `ci:updates:desktop`.
The full local run starts independent CI lanes concurrently in separate worktrees,
with clean dependencies and separate builds. `pnpm run ci:local` retains the live
summary and lane logs under `.local/ci` and displays an identified completion
notification; `pnpm run ci:status` prints progress. The Linux lane uses an unused
desktop placeholder; the Windows lane tests real signed installer bytes.

## Release setup and operation

Alpha releases are `0.0.x`, betas `0.x.y` and official releases `x.y.z`. Tags are
plain versions: the manifest and the release workflows refuse suffixes such as
`-alpha.1`. Never mark a release as a GitHub pre-release; the update feed follows
the latest full release, so a pre-release would be invisible to every installation.

Configure these GitHub environments before running release workflows:

- `release-signing`: secrets `MANIFEST_SIGNING_PRIVATE_KEY` (Ed25519 PKCS#8 PEM),
  `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; variable
  `TAURI_UPDATER_PUBLIC_KEY` (the Tauri public key). The manifest private key must
  match [releases/release.pub](../releases/release.pub). Back up both private keys
  outside GitHub and never publish them.
- `release-publish`: require a human reviewer and restrict permitted release tags.
  Protect `X.Y.Z` release tags from replacement. Give Actions package write permission and
  make both GHCR packages publicly pullable before publishing the first release.

Production private keys belong in the `release-signing` environment's encrypted
Actions secrets, with a separate secure backup. The workflow writes the manifest
key to the runner's temporary directory for signing and deletes that file afterward;
the desktop signer receives its key through the job environment. Neither private
key is included in images, installers or release assets. Local labs generate separate
`signing.pem` and `updater.key` files under their ignored `.local/thelxinoe-update-*`
directory; these are disposable test keys. Provision production keys before the
first release. If the private key matching the committed `releases/release.pub`
is unavailable, generate a new pair and replace that public key before building
the first official release.

Tauri updater signing authenticates installer updates. It is separate from Windows
Authenticode/SmartScreen signing; no Authenticode certificate is configured here.
[Tauri signing reference](https://v2.tauri.app/plugin/updater/#signing-updates).

1. Change `[workspace.package].version` in `Cargo.toml`, run
   `pnpm run version:sync`, review the change and merge it to `develop`. Released
   database schemas are frozen; a schema change ships with its migration (see
   `AGENTS.md`), and the manifest lets every schema from `MIGRATION_BASE` update.
2. When that commit's **Build and verify** run on `develop` has passed, push the
   `X.Y.Z` tag on it. **Assemble release draft** refuses any other commit instead of
   repeating the full CI. It builds the Windows installer and Linux images,
   verifies the installer signature/hash and image identities, and attaches the
   installer, `.sig`, `windows-x64.json`, signed `latest.json`, `release.pub`,
   `compose.yaml`, `env.example` and `SHA256SUMS` to a **draft** release. Images
   initially have `sha-<commit>` staging tags. Inspect release notes and assets
   while it is a draft.
3. The first release creates both GHCR packages as private. Make
   `thelxinoe-server` and `thelxinoe-controller` public (GitHub → Packages →
   package → Package settings → Change visibility); deployments pull without
   credentials.
4. Run **Publish reviewed release** from the tag after approval:
   `gh workflow run publish-release.yml --ref X.Y.Z -f tag=X.Y.Z`. It downloads and
   verifies the existing draft, checks that both images can be pulled anonymously,
   promotes those exact image digests to `X.Y.Z` and `latest`, then publishes the
   GitHub release. Nothing is rebuilt. Registry tags and GitHub publication are
   separate operations; on a partial failure retry the publish job while the
   release remains a draft.

The default feed is
`https://github.com/Valtrius/thelxinoe/releases/latest/download/latest.json`.
Both images contain `/etc/thelxinoe/release.pub`; deployments need no host key file
or release settings. `THELXINOE_RELEASE_URL` and `THELXINOE_RELEASE_KEY_FILE` remain
process-environment overrides for a private publisher or test lab. Docker overrides
belong in a separate Compose override file with an explicit `environment` section
and, for a different key, a read-only mount. A host `.env` entry alone does not pass
an unlisted variable into the container. The lab supplies these overrides itself.
`THELXINOE_MEDIA` is the server's internal media path, defaulted to `/media` by the
image; `THELXINOE_MEDIA_ROOT` selects the host directory mounted there and remains
in the deployment configuration.

Manifest validity is one year; publish a renewed release before it expires.
Do not delete images or release assets needed for installed versions and recovery.
Published versions cannot be overwritten by the assembly script. Desktop and
server API compatibility ranges are recorded in the signed manifest. A server
accepts clients speaking `API_MIN_VERSION` to `API_VERSION` (`crates/core`): raise
`API_VERSION` for a new protocol and keep `API_MIN_VERSION` until older desktops
have updated, since raising it turns them away. A desktop update waits until the
server accepts its protocol.

For a fresh deployment, download `compose.yaml` and `env.example` (saved as
`.env`) from the latest release, configure paths and identity, then
`docker compose up -d --wait`. The Compose file follows the `latest` image tags:
`docker compose pull && docker compose up -d` installs the newest release whatever
version a host runs, and the controller adopts the recreated containers. After an
in-app update the controller points the local `latest` tags at the installed
images, so a later `docker compose up -d` keeps that release. See
[operations and recovery](OPERATIONS.md#backups-and-offline-recovery).

Workflow sources: [assembly](../.github/workflows/release.yml),
[publication](../.github/workflows/publish-release.yml),
[asset verification](../scripts/release.mjs).
