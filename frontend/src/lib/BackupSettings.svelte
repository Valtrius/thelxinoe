<script lang="ts">
  import FormField from './ui/FormField.svelte';
  import {
    cardBlockClass,
    cardTableClass,
    formControlClass,
  } from './ui/styles';
  import Switch from './ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import { RefreshTask } from './refresh-task';
  import { activity, startActivity } from './activity';
  import { refreshAttention } from './attention';
  import AttentionDot from './ui/AttentionDot.svelte';
  import { errorMessage, withVerification } from './authentication';
  import { captureSession } from './session';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import Button from './ui/Button.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import Notice from './ui/Notice.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import { definitions } from './services/feature';
  import { reveal } from './ui/reveal';
  import { formatTimestamp } from './display.svelte';
  type Component = {
    key: string;
    kind: string;
    name: string;
  };
  type Exclusion = { key: string; kind: string; name: string; reason: string };
  type Backup = {
    id: string;
    stage: string;
    created_at: number;
    error: string | null;
    archive: string;
    automatic?: boolean;
    components?: Component[];
    excluded?: Exclusion[];
  };
  type Policy = { policy: 'automatic' | 'manual'; retain: number };
  type Run = {
    state: 'waiting' | 'running' | 'complete' | 'failed' | 'missed';
    updated_at: number;
    error: string | null;
  };
  const runLabels: Record<Run['state'], string> = {
    waiting: 'Waiting for an idle server',
    running: 'In progress',
    complete: 'Completed',
    failed: 'Failed',
    missed: 'Skipped',
  };
  const runTones: Record<Run['state'], 'ok' | 'warn' | 'bad' | 'info'> = {
    waiting: 'warn',
    running: 'info',
    complete: 'ok',
    failed: 'bad',
    missed: 'warn',
  };
  const ids = $props.id();
  let items = $state<Backup[]>([]),
    passphrase = $state(''),
    confirmation = $state(false),
    restoreId = $state(''),
    selected = $state<string[]>([]),
    restoreConfirmation = $state(''),
    message = $state(''),
    // Reads fail while the server restarts for an operation; they do not
    // replace what the operation reported.
    loadError = $state(''),
    busy = $state(false),
    destination = $state('');
  let mode = $state<Policy['policy']>('manual'),
    retain = $state(7),
    initialized = $state(false),
    passphraseSaved = $state(false),
    run = $state<Run | null>(null),
    maintenance = $state({ start: 3, end: 5 }),
    automaticPassphrase = $state(''),
    automaticConfirmation = $state(''),
    passphraseMessage = $state(''),
    savingPassphrase = $state(false),
    savingPolicy = $state(false),
    replacingPassphrase = $state(false);
  const hour = (value: number) => `${String(value).padStart(2, '0')}:00`;
  // Backups run in the server's maintenance window, so they use its zone.
  let timezone = $state('UTC');
  const time = (seconds: number) => formatTimestamp(seconds, timezone);
  const restoring = $derived(items.find((item) => item.id === restoreId));
  const listed = $derived(restoring?.components ?? []);
  const stageTone = (stage: string) =>
    ['complete', 'restored', 'imported'].includes(stage)
      ? 'ok'
      : stage.includes('fail')
        ? 'bad'
        : 'info';
  function label(item: { kind: string; name: string }) {
    if (item.kind === 'server') return 'Thelxinoe server';
    return definitions.find((d) => d.kind === item.kind)?.label ?? item.kind;
  }
  function chooseRestore(item: Backup) {
    restoreId = item.id;
    restoreConfirmation = '';
    selected = (item.components ?? []).map((c) => c.key);
  }
  const reads = new RefreshTask(read, 10_000);
  const load = reads.refresh;
  async function read() {
    try {
      const data = await api<{
        items: Backup[];
        destination: string;
        policy: Policy;
        passphrase_saved: boolean;
        automatic: Run | null;
        window: { start: number; end: number };
        timezone: string;
      }>('/admin/backups');
      items = data.items;
      destination = data.destination;
      passphraseSaved = data.passphrase_saved;
      run = data.automatic;
      maintenance = data.window;
      timezone = data.timezone;
      loadError = '';
      // Polling must not overwrite an edit that is still being saved.
      if (!initialized) {
        mode = data.policy.policy;
        retain = data.policy.retain;
        initialized = true;
      }
    } catch (e) {
      loadError = errorMessage(e);
    }
  }
  onMount(() => reads.start());
  async function savePassphrase() {
    const owns = captureSession();
    savingPassphrase = true;
    passphraseMessage = '';
    try {
      const saved = await withVerification(async () => {
        await api('/admin/backups/passphrase', 'POST', {
          passphrase: automaticPassphrase,
        });
      });
      if (!owns() || !saved) return;
      automaticPassphrase = '';
      automaticConfirmation = '';
      passphraseSaved = true;
      replacingPassphrase = false;
      passphraseMessage = 'Passphrase saved.';
    } catch (e) {
      if (owns()) passphraseMessage = String(e);
    } finally {
      if (owns()) savingPassphrase = false;
    }
  }
  async function create(restore = false) {
    busy = true;
    const finish = startActivity({
      target: 'backups',
      resource: restore ? restoreId : null,
      message: restore ? 'Restoring backup' : 'Creating encrypted backup',
    });
    message = '';
    try {
      const started = await api<Backup>(
        restore ? `/admin/backups/${restoreId}/restore` : '/admin/backups',
        'POST',
        {
          passphrase,
          confirm: confirmation,
          ...(restore && listed.length ? { components: selected } : {}),
        },
      );
      passphrase = '';
      restoreConfirmation = '';
      const skipped = restore ? [] : (started.excluded ?? []);
      message =
        'Operation started. The server will briefly disconnect while state is copied. Refresh after it reconnects; restoration may require signing in again.' +
        (skipped.length
          ? ` Not included and left running: ${skipped.map((e) => label(e)).join(', ')}.`
          : '');
      await load();
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
      await refreshAttention();
      finish();
    }
  }
</script>

<SettingsCard
  aria-label="Automatic backups"
  title="Automatic backups"
  description={`Run once a day when the maintenance window opens (${hour(maintenance.start)}–${hour(maintenance.end)} ${timezone}), before automatic updates. They wait up to 30 minutes for playback and background work; updates continue if the backup fails or is skipped. Change the window in Server settings.`}
  autoSave
  saving={savingPolicy}
>
  {#snippet badge()}
    {#if initialized}
      <StatusChip tone={mode === 'automatic' ? 'ok' : 'neutral'}
        >{mode === 'automatic'
          ? `Daily · ${hour(maintenance.start)}–${hour(maintenance.end)}`
          : 'Manual'}</StatusChip
      >
    {/if}
  {/snippet}
  {#if initialized}
    <AutoSaveForm
      label="Automatic backup settings"
      class="divide-y divide-line"
      bind:busy={savingPolicy}
      value={{ policy: mode, retain }}
      onRevert={(previous) => {
        mode = previous.policy;
        retain = previous.retain;
      }}
      onsave={(submitted) => api('/admin/backups/policy', 'POST', submitted)}
    >
      {#snippet children(save)}
        <SettingRow
          label="Schedule"
          description="Automatic runs daily inside the maintenance window. Manual only keeps backups you start yourself."
        >
          <ExclusiveChoiceGroup
            ariaLabel="Backup policy"
            value={mode}
            choices={[
              { value: 'automatic', label: 'Automatic' },
              { value: 'manual', label: 'Manual' },
            ]}
            onChange={(value) => {
              mode = value;
              void save();
            }}
          />
        </SettingRow>
        {#if mode === 'automatic'}
          <SettingRow
            label="Keep"
            description="Older automatic archives are deleted after a newer automatic backup succeeds. Manual and imported archives are never deleted automatically."
          >
            <input
              aria-label="Automatic backups to keep"
              class={[formControlClass, 'h-8.5! w-16! px-2! py-1! text-center']}
              type="number"
              min="1"
              max="365"
              required
              bind:value={retain}
            />
            <span class="text-xs text-muted">newest automatic archives</span>
          </SettingRow>
        {/if}
      {/snippet}
    </AutoSaveForm>
    {#if mode === 'automatic'}
      <SettingRow
        label="Passphrase"
        description={passphraseSaved
          ? 'Used for later automatic backups. Existing archives keep the passphrase they were created with.'
          : 'Save a passphrase to start automatic backups.'}
      >
        {#snippet badge()}
          <StatusChip tone={passphraseSaved ? 'ok' : 'warn'}
            >{passphraseSaved ? 'Set' : 'Not set'}</StatusChip
          >
        {/snippet}
        {#if passphraseSaved}<Button
            variant="secondary"
            size="form"
            onclick={() => (replacingPassphrase = !replacingPassphrase)}
            >{replacingPassphrase
              ? 'Keep current passphrase'
              : 'Replace passphrase…'}</Button
          >{/if}
      </SettingRow>
      {#if !passphraseSaved || replacingPassphrase}
        <form
          transition:reveal
          aria-label="Automatic backup passphrase"
          class="{cardBlockClass} grid gap-3 bg-surface-soft"
          onsubmit={(event) => {
            event.preventDefault();
            void savePassphrase();
          }}
        >
          <div
            class="grid max-w-[36rem] grid-cols-2 gap-3 compact:grid-cols-1 [&_label]:m-0"
          >
            <FormField
              >{passphraseSaved ? 'New passphrase' : 'Passphrase'}<input
                class={formControlClass}
                type="password"
                autocomplete="new-password"
                minlength="16"
                bind:value={automaticPassphrase}
              /></FormField
            >
            <FormField
              >Confirm passphrase<input
                class={formControlClass}
                type="password"
                autocomplete="new-password"
                minlength="16"
                bind:value={automaticConfirmation}
              /></FormField
            >
          </div>
          <div class="flex flex-wrap items-center gap-3">
            <Button
              size="form"
              type="submit"
              disabled={savingPassphrase ||
                automaticPassphrase.length < 16 ||
                automaticPassphrase !== automaticConfirmation}
              >{passphraseSaved
                ? 'Replace passphrase'
                : 'Save passphrase'}</Button
            >
            <span class="text-xs text-muted">At least 16 characters.</span>
            {#if automaticConfirmation && automaticPassphrase !== automaticConfirmation}<span
                class="text-xs text-muted"
              >
                The passphrases do not match.
              </span>{/if}
          </div>
          {#if passphraseMessage}<p
              role="status"
              class="m-0 text-xs text-muted"
            >
              {passphraseMessage}
            </p>{/if}
        </form>
      {:else if passphraseMessage}
        <p role="status" class="m-0 {cardBlockClass} text-xs text-muted">
          {passphraseMessage}
        </p>
      {/if}
      {#if run}
        <SettingRow
          label="Last automatic backup"
          description={`${time(run.updated_at)}${run.error ? ` · ${run.error}` : ''}`}
        >
          <StatusChip tone={runTones[run.state] ?? 'neutral'}
            >{runLabels[run.state] ?? run.state}</StatusChip
          >
        </SettingRow>
      {/if}
    {/if}
  {/if}
</SettingsCard>
<SettingsCard aria-label="Backups" title="Create a backup">
  {#snippet description()}
    <p class="mt-1 mb-0 text-xs leading-relaxed text-muted">
      Backups include server state, credentials, deployment information and
      managed-service appdata. Media and cache are excluded. Services briefly
      stop so their databases can be copied consistently. Services held by an
      ownership transfer or recovery are left running and out of the archive, as
      are the stopped originals a transfer retains.
    </p>
  {/snippet}
  <SettingRow
    label="Backup passphrase"
    description="At least 16 characters. Keep it separately; Thelxinoe cannot recover it."
    for={`${ids}-passphrase`}
    control="field"
  >
    <input
      id={`${ids}-passphrase`}
      class={formControlClass}
      type="password"
      autocomplete="new-password"
      minlength="16"
      bind:value={passphrase}
    />
  </SettingRow>
  <SettingRow
    label="I understand that services will temporarily stop."
    for={`${ids}-confirm`}
  >
    <Switch id={`${ids}-confirm`} bind:checked={confirmation} />
  </SettingRow>
  {#snippet footer()}
    {#if message}<p role="status" class="m-0 text-xs text-muted">
        {message}
      </p>{:else}<span class="text-xs text-muted"
        >Archives are saved in {destination ||
          'the controller deployment backup directory'}.</span
      >{/if}
    <div class="ml-auto flex flex-wrap gap-2">
      <Button variant="ghost" size="form" onclick={() => void load()}
        >Refresh backups</Button
      >
      <Button
        size="form"
        disabled={busy || !confirmation || passphrase.length < 16}
        onclick={() => void create()}>Create encrypted backup</Button
      >
    </div>
  {/snippet}
</SettingsCard>
<SettingsCard aria-label="Backup archives" title="Backup archives">
  {#snippet actions()}
    <span class="text-xs text-muted"
      >{items.length} {items.length === 1 ? 'archive' : 'archives'}</span
    >
  {/snippet}
  {#if items.length}
    <table class={cardTableClass}>
      <thead
        ><tr
          ><th>Created</th><th>Type</th><th>Status</th><th>File</th><th
            class="text-right"><span class="sr-only">Actions</span></th
          ></tr
        ></thead
      >
      <tbody>
        {#each items as item (item.id)}
          <tr>
            <td class="whitespace-nowrap"
              ><span class="inline-flex items-center gap-2 font-[550]"
                >{time(item.created_at)}<AttentionDot
                  activity={$activity.filter(
                    (entry) =>
                      entry.target === 'backups' && entry.resource === item.id,
                  )}
                /></span
              ></td
            >
            <td class="whitespace-nowrap"
              >{item.automatic ? 'Automatic' : 'Manual'}</td
            >
            <td
              ><StatusChip tone={stageTone(item.stage)}>{item.stage}</StatusChip
              ></td
            >
            <td class="min-w-0"
              ><span
                class="block font-mono text-[11px] wrap-anywhere text-muted"
                >{item.archive}</span
              >{#if item.error}<small class="mt-1 text-danger"
                  >{item.error}</small
                >{/if}{#each item.excluded ?? [] as skipped (skipped.key)}<small
                  class="mt-1"
                  >Not included: {label(skipped)}. {skipped.reason}</small
                >{/each}</td
            >
            <td class="text-right whitespace-nowrap">
              {#if ['complete', 'restored', 'restore-failed', 'imported'].includes(item.stage)}<Button
                  variant="secondary"
                  size="sm"
                  onclick={() => chooseRestore(item)}
                  >Restore this backup</Button
                >{/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {:else}
    <p class="m-0 {cardBlockClass} text-xs text-muted">No backups yet.</p>
  {/if}
  {#if restoreId}
    <div class="{cardBlockClass} grid gap-3 bg-surface-soft">
      {#if listed.length}
        <p class="m-0 text-xs text-muted">
          Choose what to restore. Chosen components stop and are replaced with
          the copy in this backup; changes since then are lost. Everything else
          keeps running unchanged. It does not undo changes to media or external
          services.
        </p>
        <fieldset class="grid gap-2" aria-label="Components to restore">
          {#each listed as component (component.key)}
            <Switch
              checked={selected.includes(component.key)}
              onCheckedChange={(on) =>
                (selected = on
                  ? [...selected, component.key]
                  : selected.filter((key) => key !== component.key))}
              >{label(component)}</Switch
            >
          {/each}
        </fieldset>
      {:else}
        <p class="m-0 text-xs text-muted">
          Restoring replaces server state and Thelxinoe's Owned services with
          the selected backup. Changes since that backup will be lost. Services
          still held by an ownership transfer aren't in the backup. It does not
          undo changes to media or external services.
        </p>
      {/if}
      <FormField class="mb-0 max-w-136"
        >Type RESTORE to confirm<input
          class={formControlClass}
          bind:value={restoreConfirmation}
          autocomplete="off"
        /></FormField
      >
      <div class="flex flex-wrap gap-2">
        <Button
          variant="danger"
          size="form"
          disabled={busy ||
            !confirmation ||
            passphrase.length < 16 ||
            restoreConfirmation !== 'RESTORE' ||
            (listed.length > 0 && selected.length === 0)}
          onclick={() => void create(true)}>Restore selected state</Button
        ><Button
          variant="secondary"
          size="form"
          onclick={() => (restoreId = '')}>Cancel</Button
        >
      </div>
      <p class="m-0 text-xs text-muted">
        Enter the backup passphrase and confirm the service stop in the card
        above before restoring.
      </p>
    </div>
  {/if}
  {#if loadError}
    <div class={cardBlockClass}>
      <Notice role="status" class="mb-0"
        >Couldn't refresh backups: {loadError}</Notice
      >
    </div>
  {/if}
</SettingsCard>
