<script lang="ts">
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import Switch from './ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import { withVerification } from './authentication';
  import { captureSession } from './session';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import Button from './ui/Button.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import Notice from './ui/Notice.svelte';
  import Panel from './ui/Panel.svelte';
  import { rowClass } from './ui/styles';
  let { timezone, timeFormat } = $props<{
    timezone: string;
    timeFormat: '12h' | '24h';
  }>();
  type Backup = {
    id: string;
    stage: string;
    created_at: number;
    error: string | null;
    archive: string;
    automatic?: boolean;
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
  let items = $state<Backup[]>([]),
    passphrase = $state(''),
    confirmation = $state(false),
    restoreId = $state(''),
    restoreConfirmation = $state(''),
    message = $state(''),
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
    savingPassphrase = $state(false);
  const hour = (value: number) => `${String(value).padStart(2, '0')}:00`;
  const time = (seconds: number) =>
    new Date(seconds * 1000).toLocaleString(undefined, {
      timeZone: timezone,
      hour12: timeFormat === '12h',
    });
  async function load() {
    try {
      const data = await api<{
        items: Backup[];
        destination: string;
        policy: Policy;
        passphrase_saved: boolean;
        automatic: Run | null;
        window: { start: number; end: number };
      }>('/admin/backups');
      items = data.items;
      destination = data.destination;
      passphraseSaved = data.passphrase_saved;
      run = data.automatic;
      maintenance = data.window;
      // Polling must not overwrite an edit that is still being saved.
      if (!initialized) {
        mode = data.policy.policy;
        retain = data.policy.retain;
        initialized = true;
      }
    } catch (e) {
      message = String(e);
    }
  }
  onMount(() => {
    void load();
    const timer = setInterval(() => void load(), 10000);
    return () => clearInterval(timer);
  });
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
      passphraseMessage = 'Passphrase saved.';
    } catch (e) {
      if (owns()) passphraseMessage = String(e);
    } finally {
      if (owns()) savingPassphrase = false;
    }
  }
  async function create(restore = false) {
    busy = true;
    message = '';
    try {
      await api(
        restore ? `/admin/backups/${restoreId}/restore` : '/admin/backups',
        'POST',
        { passphrase, confirm: confirmation },
      );
      passphrase = '';
      restoreConfirmation = '';
      message =
        'Operation started. The server will briefly disconnect while state is copied. Refresh after it reconnects; restoration may require signing in again.';
      await load();
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Panel aria-label="Automatic backups">
  <h2>Automatic backups</h2>
  <p>
    Automatic backups run once a day when the server maintenance window opens ({hour(
      maintenance.start,
    )}–{hour(maintenance.end)}
    {timezone}), before automatic updates. They wait up to 30 minutes for
    playback and background work to finish; updates continue if the backup fails
    or is skipped. Change the window in Server settings.
  </p>
  {#if initialized}
    <AutoSaveForm
      label="Automatic backup settings"
      class="flex flex-wrap items-center gap-x-3 gap-y-2"
      value={{ policy: mode, retain }}
      onRevert={(previous) => {
        mode = previous.policy;
        retain = previous.retain;
      }}
      onsave={(submitted) => api('/admin/backups/policy', 'POST', submitted)}
    >
      {#snippet children(save)}
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
        {#if mode === 'automatic'}
          <label class="flex items-center gap-2 text-xs text-muted"
            >Keep<input
              aria-label="Automatic backups to keep"
              class={[formControlClass, 'h-8! w-16! px-2! py-1!']}
              type="number"
              min="1"
              max="365"
              required
              bind:value={retain}
            />newest automatic archives</label
          >
        {/if}
      {/snippet}
    </AutoSaveForm>
    {#if mode === 'automatic'}
      <p>
        Older automatic archives are deleted after a newer automatic backup
        succeeds. Manual and imported archives are never deleted automatically.
      </p>
      {#if passphraseSaved}
        <p>
          A passphrase is saved for automatic backups. Replacing it only affects
          later backups; existing archives keep the passphrase they were created
          with.
        </p>
      {:else}
        <Notice tone="warning"
          ><p>Save a passphrase to start automatic backups.</p></Notice
        >
      {/if}
      <form
        aria-label="Automatic backup passphrase"
        class="my-4 grid max-w-136 gap-3"
        onsubmit={(event) => {
          event.preventDefault();
          void savePassphrase();
        }}
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
        <div>
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
        </div>
        {#if automaticConfirmation && automaticPassphrase !== automaticConfirmation}<p
            class="text-muted"
          >
            The passphrases do not match.
          </p>{/if}
        {#if passphraseMessage}<p role="status">{passphraseMessage}</p>{/if}
      </form>
      {#if run}<p>
          <strong>Last automatic backup</strong>
          <small class="block"
            >{time(run.updated_at)} · {runLabels[run.state] ?? run.state}</small
          >{#if run.error}<small class="block wrap-anywhere">{run.error}</small
            >{/if}
        </p>{/if}
    {/if}
  {/if}
</Panel>
<Panel aria-label="Backups">
  <h2>Backups and restore</h2>
  <p>
    Backups include server state, credentials, deployment information and
    managed-service appdata. Media and cache are excluded. Services briefly stop
    so their databases can be copied consistently.
  </p>
  <p>
    Archives are encrypted with your passphrase and saved in {destination ||
      'the controller deployment backup directory'}. Keep the passphrase
    separately; Thelxinoe cannot recover it.
  </p>
  <FormField class="my-4 block max-w-136"
    >Backup passphrase<input
      class={formControlClass}
      type="password"
      autocomplete="new-password"
      minlength="16"
      bind:value={passphrase}
    /></FormField
  >
  <Switch bind:checked={confirmation}
    >I understand that services will temporarily stop.</Switch
  >
  <Button
    size="form"
    class="m-[0.3rem]"
    disabled={busy || !confirmation || passphrase.length < 16}
    onclick={() => void create()}>Create encrypted backup</Button
  >
  <Button
    variant="secondary"
    size="form"
    class="m-[0.3rem]"
    onclick={() => void load()}>Refresh backups</Button
  >
  {#if message}<p role="status">{message}</p>{/if}
</Panel>
<Panel class="settings-wide" aria-label="Backup archives">
  <h2>Backup archives</h2>
  {#each items as item (item.id)}<div class={rowClass}>
      <div>
        <strong
          >{time(item.created_at)} · {item.stage}{item.automatic
            ? ' · automatic'
            : ''}</strong
        ><small class="block wrap-anywhere">{item.archive}</small
        >{#if item.error}<p>{item.error}</p>{/if}
      </div>
      {#if ['complete', 'restored', 'restore-failed', 'imported'].includes(item.stage)}<Button
          variant="secondary"
          size="form"
          class="m-[0.3rem]"
          onclick={() => {
            restoreId = item.id;
            restoreConfirmation = '';
          }}>Restore this backup</Button
        >{/if}
    </div>{:else}<p class="text-muted">No backups yet.</p>{/each}
  {#if restoreId}<div class="border border-line p-4">
      <p>
        Restoring replaces server and managed-service state with the selected
        backup. Changes since that backup will be lost. It does not undo changes
        to media or external services.
      </p>
      <FormField class="my-4 block max-w-136"
        >Type RESTORE to confirm<input
          class={formControlClass}
          bind:value={restoreConfirmation}
          autocomplete="off"
        /></FormField
      ><Button
        variant="secondary"
        size="form"
        class="m-[0.3rem]"
        disabled={busy ||
          !confirmation ||
          passphrase.length < 16 ||
          restoreConfirmation !== 'RESTORE'}
        onclick={() => void create(true)}>Restore selected state</Button
      ><Button
        variant="secondary"
        size="form"
        class="m-[0.3rem]"
        onclick={() => (restoreId = '')}>Cancel</Button
      >
    </div>{/if}
</Panel>
