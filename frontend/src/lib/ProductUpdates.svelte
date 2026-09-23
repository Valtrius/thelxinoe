<script lang="ts">
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import UpdatePolicyFields from './ui/UpdatePolicyFields.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  let { timeFormat } = $props<{ timeFormat: '12h' | '24h' }>();
  type Update = {
    id: string;
    version: string;
    previous_version: string;
    stage: string;
    error: string | null;
    snapshot_ready: boolean;
    recovery_tested: boolean;
    activation_crossed: boolean;
  };
  type Status = {
    version: string;
    timezone: string;
    configured: boolean;
    policy: { policy: string; window_start: number; window_end: number };
    release: {
      version: string;
      notes: string;
      migration: { recovery: string };
    } | null;
    observation: { checked_at: number; error: string | null } | null;
    controller: { items: Update[]; error?: string };
  };
  let status = $state<Status | null>(null),
    message = $state(''),
    busy = $state(false),
    restore = $state('');
  let policy = $state('notify'),
    start = $state(3),
    end = $state(5);
  async function load(initial = false) {
    try {
      status = await api<Status>('/admin/product-update');
      if (initial) {
        policy = status.policy.policy;
        start = status.policy.window_start;
        end = status.policy.window_end;
      }
    } catch (e) {
      message = String(e);
    }
  }
  onMount(() => {
    void load(true);
    const timer = setInterval(() => void load(), 10000);
    return () => clearInterval(timer);
  });
  async function command(path: string, body: unknown = {}) {
    busy = true;
    message = '';
    try {
      await api('/admin/product-update/' + path, 'POST', body);
      message =
        'Request accepted. Preparation and installation can briefly disconnect the server; refresh after it reconnects.';
      await load();
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section
  aria-label="Product updates"
  class="mt-6 grid gap-3 border-t border-line pt-5"
>
  <h3>Server updates</h3>
  {#if status}
    <AutoSaveForm
      label="Server update settings"
      class="grid gap-3"
      value={{ policy, window_start: start, window_end: end }}
      onRevert={(previous) => {
        policy = previous.policy;
        start = previous.window_start;
        end = previous.window_end;
      }}
      onsave={(submitted) =>
        api('/admin/product-update/policy', 'POST', submitted)}
    >
      {#snippet children(save)}
        <UpdatePolicyFields
          bind:policy
          bind:start
          bind:end
          timezone={status?.timezone ?? 'UTC'}
          onChange={() => void save()}
        />
      {/snippet}
    </AutoSaveForm>
    {#if !status.configured}<p class="text-xs text-muted">
        Automatic checks will start once a signed release channel is configured.
      </p>
    {:else if status.observation?.checked_at && !status.observation.error && !status.release}<p
        class="text-xs text-muted"
      >
        Up to date.
      </p>
    {:else if !status.observation?.checked_at}<p class="text-xs text-muted">
        Waiting for the first automatic update check.
      </p>{/if}
    {#if status.observation?.checked_at}<p>
        Last check: {new Date(
          status.observation.checked_at * 1000,
        ).toLocaleString(undefined, {
          timeZone: status.timezone,
          hour12: timeFormat === '12h',
        })}
      </p>{/if}
    {#if status.observation?.error}<p role="status">
        {status.observation.error}
      </p>{/if}
    {#if status.release}
      <h3>Version {status.release.version}</h3>
      <p class="whitespace-pre-wrap">{status.release.notes}</p>
      <p>
        Declared recovery: {status.release.migration.recovery}. Thelxinoe
        retains a full verified snapshot before activation.
      </p>
      <Button
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() => command('prepare')}>Prepare and test release</Button
      >
    {/if}
    {#each status.controller.items as item (item.id)}
      <div class="border-t border-line py-4">
        <strong>{item.previous_version} → {item.version} · {item.stage}</strong>
        <p>
          Rollback snapshot: {item.snapshot_ready
            ? 'verified'
            : 'not captured yet'} · Recovery test: {item.recovery_tested
            ? 'passed'
            : 'not complete'}
        </p>
        {#if item.error}<p>{item.error}</p>{/if}
        {#if item.stage === 'ready'}<Button
            size="form"
            disabled={busy}
            onclick={() => command(`${item.id}/activate`, { confirm: true })}
            >Install prepared release</Button
          >{/if}
        {#if item.snapshot_ready && ['committed', 'runtime-failure', 'recovery-required'].includes(item.stage)}
          <p>
            Restoring returns all server state to its pre-update snapshot.
            Changes since the update are lost. It does not undo media or
            external service changes.
          </p>
          <FormField
            >Type RESTORE to recover version {item.previous_version}<input
              class={formControlClass}
              bind:value={restore}
              autocomplete="off"
            /></FormField
          >
          <Button
            variant="secondary"
            size="form"
            disabled={busy || restore !== 'RESTORE'}
            onclick={() => command(`${item.id}/recover`, { confirm: true })}
            >Restore previous release and state</Button
          >
        {/if}
      </div>
    {/each}
    {#if status.controller.error}<p>{status.controller.error}</p>{/if}
  {/if}
  {#if message}<p role="status">{message}</p>{/if}
</section>
