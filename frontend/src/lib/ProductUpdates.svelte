<script lang="ts">
  import Switch from './providers/components/ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
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
    confirmed = $state(false),
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

<section class="panel" aria-label="Product updates">
  <h2>Thelxinoe updates</h2>
  <p>
    Server, controller and web share one release. Windows installs its matching
    desktop update separately.
  </p>
  {#if status}
    <p>Installed version {status.version}</p>
    {#if !status.configured}<p>
        Configure a release channel and its signing public key in the deployment
        to enable release checks.
      </p>{/if}
    <div class="controls">
      <label
        >Update policy<select bind:value={policy}
          ><option value="notify">Notify</option><option value="automatic"
            >Automatic</option
          ><option value="manual">Manual</option></select
        ></label
      >
      <label
        >Maintenance starts (UTC)<input
          type="number"
          min="0"
          max="23"
          bind:value={start}
        /></label
      >
      <label
        >Maintenance ends (UTC)<input
          type="number"
          min="0"
          max="23"
          bind:value={end}
        /></label
      >
    </div>
    <p>
      Automatic updates wait for idle playback and background work and require a
      successfully tested recovery path. Equal start and end hours allow any
      time.
    </p>
    <button
      class="secondary"
      disabled={busy}
      onclick={() =>
        command('policy', { policy, window_start: start, window_end: end })}
      >Save update policy</button
    >
    <button
      class="secondary"
      disabled={busy || !status.configured}
      onclick={() => command('check')}>Check signed releases</button
    >
    {#if status.observation?.checked_at}<p>
        Last check: {new Date(
          status.observation.checked_at * 1000,
        ).toLocaleString()}
      </p>{/if}
    {#if status.observation?.error}<p role="status">
        {status.observation.error}
      </p>{/if}
    {#if status.release}
      <h3>Version {status.release.version}</h3>
      <p class="notes">{status.release.notes}</p>
      <p>
        Declared recovery: {status.release.migration.recovery}. Thelxinoe
        retains a full verified snapshot before activation.
      </p>
      <button
        class="secondary"
        disabled={busy}
        onclick={() => command('prepare')}>Prepare and test release</button
      >
    {/if}
    <Switch bind:checked={confirmed}
      >I understand that installation briefly stops the server.</Switch
    >
    {#each status.controller.items as item (item.id)}
      <div class="update">
        <strong>{item.previous_version} → {item.version} · {item.stage}</strong>
        <p>
          Rollback snapshot: {item.snapshot_ready
            ? 'verified'
            : 'not captured yet'} · Recovery test: {item.recovery_tested
            ? 'passed'
            : 'not complete'}
        </p>
        {#if item.error}<p>{item.error}</p>{/if}
        {#if item.stage === 'ready'}<button
            class="primary"
            disabled={busy || !confirmed}
            onclick={() => command(`${item.id}/activate`, { confirm: true })}
            >Install prepared release</button
          >{/if}
        {#if item.snapshot_ready && ['committed', 'runtime-failure', 'recovery-required'].includes(item.stage)}
          <p>
            Restoring returns all server state to its pre-update snapshot.
            Changes since the update are lost. It does not undo media or
            external service changes.
          </p>
          <label
            >Type RESTORE to recover version {item.previous_version}<input
              bind:value={restore}
              autocomplete="off"
            /></label
          >
          <button
            class="secondary"
            disabled={busy || restore !== 'RESTORE'}
            onclick={() => command(`${item.id}/recover`, { confirm: true })}
            >Restore previous release and state</button
          >
        {/if}
      </div>
    {/each}
    {#if status.controller.error}<p>{status.controller.error}</p>{/if}
  {/if}
  {#if message}<p role="status">{message}</p>{/if}
</section>

<style>
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
  }
  .controls label {
    min-width: 12rem;
  }
  .update {
    border-top: 1px solid var(--line);
    padding: 1rem 0;
  }
  .notes {
    white-space: pre-wrap;
  }
  button {
    margin: 0.3rem;
  }
</style>
