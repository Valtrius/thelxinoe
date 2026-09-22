# Releases and recovery

The server, controller, web bundle and Windows application share a product version. API compatibility is checked separately: desktop and web clients send their API version, and incompatible clients show an update-required screen. Public signed release metadata remains available before login so an old desktop can update.

## Publisher trust

`releases/release.pub` is the base64 Ed25519 publisher public key. The server and controller read a read-only mount of that key; the desktop embeds it at build time. A server administrator cannot replace the desktop's publisher trust through a release response. Private signing keys stay outside source control and are never served by the release host.

Set `THELXINOE_RELEASE_URL` to an HTTPS signed envelope. The server requires HTTPS, rejects redirects, bounds response size and checks the signature before storing a candidate. Notify is the default policy. Manual only checks on request. Automatic waits for the UTC maintenance window, idle playback/background work and successful isolated recovery testing. A failed or restored version remains blocked from Automatic retry in the controller journal, even when server SQLite is restored.

An envelope contains `payload` and `signature`, both base64. The signature covers `Thelxinoe release manifest v1` followed by a NUL byte and the exact payload bytes. The payload declares immutable Linux amd64 server/controller references and platform image identities, a Windows artifact URL/hash/byte count/Tauri signature/public key, API compatibility, source and target schema versions, recovery mode, protocol, validity times and notes. Stable versions, validity, migration range and supported recovery protocol are checked again at activation.

## Building a release

1. Update the shared Cargo version and run `npm run version:sync`. The unreleased database has one fresh schema in `crates/database/schema.sql`, at schema version 1. There is no production upgrade runner. Releases using that schema declare the same source and target schema; a future incompatible schema requires an explicit upgrade design before it can be offered to existing installations.
2. Run the [release checklist](RELEASE_CHECKLIST.md) using the commands in [TESTING.md](TESTING.md). Build Linux server/controller images and push to the chosen registry; record their repository manifest digests and platform config digests.
3. Build Windows with `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` supplied privately. Use a Tauri configuration override containing `bundle.createUpdaterArtifacts: true` and `plugins.updater.pubkey` with the corresponding Tauri public key. Keep this signing key distinct from the Ed25519 manifest key. The resulting NSIS installer has an adjacent `.sig` file.
4. Prepare a reviewed manifest draft containing the release/image/API/migration metadata and the final HTTPS `windows_x64.url`.
5. Run `node scripts/assemble-release.mjs draft.json installer.exe installer.exe.sig updater.pub manifest.json`. This records the actual artifact hash/size/signature and writes sibling `windows-x64.json` metadata. Publish that metadata beside the installer URL.
6. Run `node scripts/sign-release.mjs manifest.json private-manifest-key.pem latest.json`. Publish the installer, desktop metadata and signed envelope. Keep the manifest signing key backed up privately.

The Windows client independently checks the signed manifest, expiry and API range. It requires Tauri metadata to match the signed artifact URL/version/signature, bounds the download, verifies the Tauri signature and compares the actual byte count and SHA-256 before starting the installer. Installation stops MPV and exits the application. The installer restarts the updated application.

No public release host or registry publication is configured in this checkout. The local validation channel uses a private registry and a separate test publisher key. Production builds use the committed publisher public key, whose private counterpart is kept outside Git.

## Container activation and offline recovery

Preparation retains the original images, briefly stops the server for a consistent snapshot, verifies SQLite integrity/foreign keys/schema and the original credential key, then starts the original server again. The successor server validates disposable copied state with networking disabled, no Docker socket and no production mounts. The recovery worker then restores that clone and runs the original server's validation command. A successor controller probe must also pass. The private release fixture injects a schema change only into its disposable source copy to test recovery across incompatible versions.

Activation captures a new verified rollback bundle and validates real state while the server is stopped and isolated. Only after validation does the controller create the successor server and perform a fenced controller handoff. Both runtime and persistent deployment file locks protect every Docker mutation. The accepted descriptor and Compose pins advance together through one atomic pointer. The successor records the activation boundary before starting the server with production access. Failures after that boundary require explicit recovery; restoration cannot undo media or external service side effects.

The controller's private Unix API works while the HTTP server is unavailable. From the accepted controller container, list operations with:

```sh
curl --unix-socket /run/thelxinoe/controller.sock http://localhost/stack/product
```

After selecting the correct snapshot, explicitly restore its retained server state and old image/spec generation:

```sh
curl --unix-socket /run/thelxinoe/controller.sock \
  -H 'Content-Type: application/json' -d '{"confirm":true}' \
  http://localhost/stack/product/OPERATION_UUID/recover
```

Recovery reads controller journals and retained files/images; it does not start the incompatible server or parse its migrated database. Preserve the deployment directory, server state and retained recovery images. Do not prune recovery images before backup/recovery verification.

Use the generated deployment `compose.yaml` and `compose.override.yaml` together for recreation. The override pins accepted immutable images, even if a bootstrap file names an older version. The controller accepts new Compose container IDs only after comparing images, names, environment, commands, host access, mounts and networks, then records a new generation. Starting a stale controller cannot acquire the accepted writer role.

Encrypted archives can restore an earlier accepted server/controller generation when its historical descriptor, retained images and managed-service layout are still available. The authenticated archive descriptor must match the controller's historical record. Restoring into a different host/layout requires restoring the deployment directory and original mount/network layout first; arbitrary archive-supplied Docker configurations are rejected.

## Thelxinoe updates

One monorepo release version covers server, controller, web, and Tauri artifacts.

The release publishes a signed manifest containing product version, server image digest, controller image digest, desktop artifact hashes, API compatibility metadata, database migration metadata, and rollback-safety metadata.

The server update policy is Automatic, Notify, or Manual, with Notify as default.

The Docker controller applies the server release from the persistent first-party deployment descriptor. Controller replacement uses the fenced self-handoff protocol described in the runtime topology so the existing controller remains the only Docker writer until the successor path is proven healthy and the new generation is committed.

Before an update, the running server creates a verified rollback bundle. SQLite is captured through its supported online backup mechanism or a quiesced database copy rather than by copying live WAL files. The controller records the old deployment descriptor and retains the previous images before stopping the old server.

The controller owns an offline restore path that does not require the new server binary or the migrated database to start. Release migration metadata declares whether rollback can reuse the migrated state or requires restoration of the pre-update bundle. A forward-only database migration is therefore not automatically disqualifying, but Automatic installation is allowed only when the current controller understands and has successfully prepared a tested unattended recovery path for that release. If no unattended recovery path exists, the release can only be installed after explicit administrator action.

During server validation, the successor server runs in a pre-activation mode with persistent scheduled jobs and external side-effect work disabled, no privileged Docker access, and no live integration egress. Its migration, local-state, and health checks complete before production activation. If validation fails before activation, the controller restores the old deployment descriptor, old images, and pre-update server state offline. The update is not considered committed until those checks and the controller handoff have completed.

Automatic server updates wait for active playback, transcodes, downloads, and other affected work to finish.

Tauri updates separately from the same product release. Client/server compatibility depends on the API contract, not exact version equality.
