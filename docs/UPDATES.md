# Updates

Server and Windows desktop share one product version. GitHub Releases supplies
the signed publisher envelope and Windows installer; GHCR supplies Linux amd64
server/controller images pinned by digest. Web assets travel with the server.

Server **Notify** discovers releases and adds an administrator notification that
opens Settings → Server. **Automatic** also prepares and installs during the
configured idle maintenance window. Preparation tests copied state and recovery;
installation replaces both containers. Failed activation restores the snapshot.
An open web client reconnects and offers to reload its bundle after an upgrade.
Restoring a snapshot also resets the event connection and refreshes client state.

Desktop checks the publisher directly, including before login or while the server
is unavailable. **Automatic** downloads and verifies the installer. **Restart and
install** is explicit and requires playback to stop. A reachable incompatible
server blocks installation. Downloads are held for the current desktop session.
See [server policy](../apps/server/src/product.rs),
[controller recovery](../apps/docker-controller/src/product.rs) and
[desktop updater](../apps/desktop/src/updates.rs).

## Manual local lab

Install the normal build prerequisites, Docker with Linux containers, Node 24,
Rust, and OpenSSL (Git for Windows supplies it). Run `npm ci` first. Windows builds
also need the Tauri prerequisites and WebView2. The first run builds two complete
versions and can take several minutes.

```sh
npm run updates:lab -- start
# Docker/web only (also works on Linux):
npm run updates:lab -- start --server-only
# Windows installer only, with a local native API server and no Docker:
npm run updates:lab -- start --desktop-only
```

The command prints the publisher URL, server URL and `lab.json` path. The Windows
lab opens a separately named desktop with its own installation, keyring identity
and WebView profile. It starts at the repository version and offers the next patch
version. Test keys, source copies and artifacts stay under `.local/<lab-id>`.
Production sources, trust keys and product version are not changed by the lab.

1. Open the printed server URL and create `admin` with password
   `update lab passphrase`. Connect the lab desktop to the same URL and sign in.
2. Open the printed HTTPS publisher URL. Accept the browser's warning for this
   disposable local certificate; the lab apps trust its generated CA explicitly.
   Select **candidate** and apply. Alternatively:
   `npm run updates:lab -- candidate "<lab.json>"`.
3. In either web or desktop, open Settings → Server → **Check server release**.
   Follow the notification's **Open server updates** action. Use **Prepare and
   test release**, then **Install prepared release**. Keep both clients open to
   review progress, reconnection and the web reload notice.
4. In the desktop notice, choose **View desktop update**, then **Download desktop
   update** and **Restart and install desktop update**. Check the displayed version
   and retained login after the installer relaunches it.
5. Publisher choices also exercise unavailable/expired/forged feeds and mismatched
   metadata/corrupted installers. Use **Check** after changing the publisher.
   Review playback gating with actual playback in the lab if relevant to UI work.

To review server recovery, first prepare a release, then run
`npm run updates:lab -- fail-validation "<lab.json>"` before installing it.
The candidate deliberately fails validation and the controller restores the
snapshot. Run `npm run updates:lab -- clear-failure "<lab.json>"` before retrying.

```sh
npm run updates:lab -- reset "<lab.json>"
npm run updates:lab -- stop "<lab.json>"
```

Reset recreates the Docker baseline and reinstalls the base desktop. Complete
server setup again after a Docker reset. In desktop-only mode it keeps the native
server data. Stop terminates lab processes and removes its Docker containers and
volumes; bind-mounted state, build artifacts and the isolated Windows
installation/profile are kept. Each Docker reset uses a new state directory.
The lab publisher listens on the host so Docker can reach it; its control endpoint
accepts loopback requests only. No private signing key is mounted in containers.

## Automated qualification

```sh
npm run test:updates                 # Windows: Docker + native desktop
npm run test:updates -- --server-only # Linux or Windows
npm run test:updates -- --desktop-only
```

These use actual containers, browsers, signed NSIS installers and a local HTTPS
publisher. The Docker suite exercises failed validation, interrupted activation
with the registry offline, successful installation, Compose recreation and full
state restore. The Windows suite exercises independent discovery, trust failures,
compatibility rejection, installation and retained credentials. UI actions cover
the successful flows; assertions check functional results, not layout or timing.

Each run saves `test-results/updates/<timestamp>/result.json`, screenshots,
Playwright traces and operation/process logs. Open a trace with
`npx playwright show-trace <trace.zip>`. `--keep` retains the lab for inspection;
`--lab "<lab.json>"` uses an existing lab after a reset. Generated credentials are
test-only. Failure points are injected solely into disposable candidate sources.
No production migrations or test switches are added.

The regular CI includes separate Linux server and Windows desktop lanes. Local
`npm run ci` runs the combined suite on Windows. The Linux lane uses an unused
desktop placeholder; the Windows lane tests real signed installer bytes.

## Release setup and operation

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
   `npm run version:sync`, review the change and complete local CI. Before the first
   official release, source and target DB schemas remain equal; no migration is
   created by this workflow.
2. Push the corresponding `X.Y.Z` tag when authorized. **Assemble release draft**
   runs the full CI, builds the Windows installer and Linux images, verifies the
   installer signature/hash and image identities, and attaches the installer,
   `.sig`, `windows-x64.json`, signed `latest.json`, `release.pub`, digest-pinned
   `compose.yaml` and `SHA256SUMS` to a **draft** release. Images initially have
   `sha-<commit>` staging tags. Inspect release notes and assets while it is a draft.
3. Run **Publish reviewed release** with that tag after approval. It downloads and
   verifies the existing draft, promotes those exact image digests to `X.Y.Z` and
   `latest`, then publishes the GitHub release. Nothing is rebuilt. Registry tags
   and GitHub publication are separate operations; on a partial failure retry the
   publish job while the release remains a draft.

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
server API compatibility ranges are recorded in the signed manifest; any future
breaking protocol change needs a compatible intermediate release.

For a fresh released deployment, download `compose.yaml` from
that release, configure paths/identity using
[.env.example](../.env.example), then `docker compose up -d --wait`. After an
in-app update, recreate from the controller's generated deployment directory and
its `compose.override.yaml`, which pins the accepted generation. See
[operations and recovery](OPERATIONS.md#backups-and-offline-recovery).

Workflow sources: [assembly](../.github/workflows/release.yml),
[publication](../.github/workflows/publish-release.yml),
[asset verification](../scripts/release.mjs).
