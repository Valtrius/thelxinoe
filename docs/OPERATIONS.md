# Administration and backups

Settings shows state/cache usage, active playback and transcodes, integration observations, indexer/download health, recent failed jobs and all devices. Support-service observations refresh in the background. The existing support-service panels provide queue details and actions. Diagnostic export uses an explicit allowlist of version, schema, database-check outcome and job counts; it never exports settings, paths, tokens, arbitrary error strings or viewing history.

Notifications are stored per user for 90 days and delivered to connected browser/PWA/Windows clients through the existing authenticated event stream. Repeated observations of one unresolved fault are deduplicated; recovery followed by another failure creates a new notification. Administrators receive operational alerts. Users receive request-status and account-reconnection notices. Background Web Push remains outside v1.

Administrators can change roles/passwords, revoke devices and delete other users. Changes revoke existing sessions. Database constraints preserve at least one administrator. Deletion cascades through private media state, queues, playlists, history and linked credentials; removes private events and retention selections; and preserves installed services by transferring their administrative reference. Shared media stays in place. Historical encrypted backups retain their captured state and may restore deleted accounts.

## Backup operation

The Backups panel takes an administrator-supplied passphrase of at least 16 bytes. The controller stops the server and managed services, captures their state through the same constrained copy worker used by service updates, restarts them and encrypts the snapshot. Media and cache are excluded. Failed components cause the operation to fail; the application never labels a partial archive complete.

Archives use the [age passphrase format](https://docs.rs/age/latest/age/struct.Encryptor.html) around TAR, with bounded size/entry counts. Restore rejects links, special files, traversal and duplicate names and verifies the authenticated end of the encrypted stream before touching live state. Passphrases are not stored in operation journals or SQLite.

The main Compose file mounts `THELXINOE_BACKUP_ROOT` (default `.local/backups`) on the controller at `/backups`. Without that configuration archives default to `<controller deployment>/backups/<id>.age`. Internal staging and recovery snapshots remain under the private deployment tree. Copying an archive with its original UUID filename into the configured destination makes it available for import. Keep its passphrase separately. External replication and retention of backup files are the operator's responsibility.

The controller requires `CHOWN`, `FOWNER` and `DAC_OVERRIDE` to preserve and read protected component files. These are declared in the main Compose file. It remains on `network_mode: none` with a read-only root filesystem and its private Unix socket. The server never receives Docker credentials or these capabilities.

## Restore and recovery

The UI requires explicit confirmation before restoring. The entire archive is decrypted into private staging first. The controller validates the accepted deployment generation, images, service specifications and component paths. It then stops the components and takes a recovery snapshot of the current state before replacing anything.

If replacement fails before restart, it restores that recovery snapshot. The journal survives controller termination. On restart the controller cleans up its interrupted workers, restores pre-operation state when necessary and starts the original components. Once restored services are restarted, external work may resume; the controller does not automatically rewind those external effects.

Archives include the full first-party descriptor, Compose pins and managed-service specifications. Restore can recreate an older server/controller generation using its previously accepted descriptor and retained images, including after a forward-only server migration. The old server validates a disposable copy before handoff. The managed-service layout must still match; unrelated adoption or changed paths block restoration instead of guessing host locations. Preserve the deployment directory and retained images alongside portable archives when moving hosts. See [release recovery](RELEASES.md) for the private offline recovery API.

## Evidence

`compose.operations.test.yaml` is an isolated HTTPS deployment. `node scripts/test-operations.mjs` exercises encrypted backup, wrong-password rejection, server-state restoration, Radarr appdata restoration and API reconnection. `node scripts/test-backup-interruption.mjs` terminates that test controller during snapshot and restore, then verifies restart and rollback of the incomplete restore. The scripts must only target their dedicated generated test deployment.

Private results: `.local/operations-result.json`, `.local/backup-interruption-result.json`, `.local/operations.png`. Private test passphrases and archives are excluded from Git. Rust tests check notification isolation/deduplication, diagnostic redaction, user cleanup, archive corruption, traversal, links and duplicate entries. Browser notification and administration workflows are checked independently of the backup interruption tests.
