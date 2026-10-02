<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../api';
  import RecyclarrConfiguration from './RecyclarrConfiguration.svelte';
  import AutoSaveForm from '../ui/AutoSaveForm.svelte';
  import FormField from '../ui/FormField.svelte';
  import Switch from '../ui/Switch.svelte';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import { formControlClass } from '../ui/styles';

  type Snapshot = {
    settings: { provision_id: string; paused: boolean; hour: number } | null;
    runs: { id: string; state: string; error: string | null }[];
    timezone: string;
  };
  let snapshot = $state<Snapshot | null>(null),
    error = $state(''),
    loadError = $state(''),
    busy = $state(false),
    loaded = $state(false),
    schedule = $state({ paused: false, hour: 4 });
  let feedbackRun = '';
  async function refresh() {
    try {
      snapshot = await api<Snapshot>('/admin/recyclarr');
      loadError = '';
      if (!loaded && snapshot.settings) {
        schedule = {
          paused: snapshot.settings.paused,
          hour: snapshot.settings.hour,
        };
        loaded = true;
      }
      const run = snapshot.runs.find((run) => run.id === feedbackRun);
      if (run && !['queued', 'running', 'retrying'].includes(run.state)) {
        busy = false;
        feedbackRun = '';
        error =
          run.error ??
          (run.state === 'complete'
            ? ''
            : 'Sync did not finish. Retry the sync.');
      }
    } catch (caught) {
      loadError = String(caught);
    }
  }
  async function sync() {
    busy = true;
    error = '';
    try {
      const queued = await api<{ id: string }>(
        '/admin/recyclarr/sync',
        'POST',
        {},
      );
      feedbackRun = queued.id;
      await refresh();
    } catch (caught) {
      busy = false;
      error = String(caught);
    }
  }
  onMount(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 4000);
    return () => clearInterval(timer);
  });
</script>

<section
  class="grid gap-4 border-b border-line py-4"
  aria-label="Recyclarr guide configuration"
>
  <div class="flex flex-wrap items-center justify-between gap-3">
    <strong class="text-xs">TRaSH Guides</strong>
    <Button size="form" loading={busy} onclick={() => void sync()}
      >Sync now</Button
    >
  </div>
  {#if error || loadError}<Notice tone="danger" role="alert"
      >{error || loadError}</Notice
    >{/if}
  {#if loaded && snapshot?.settings}
    <RecyclarrConfiguration provisionId={snapshot.settings.provision_id} />
    <AutoSaveForm
      value={schedule}
      label="Daily guide sync"
      class="grid gap-3"
      onsave={async (value) => {
        await api('/admin/recyclarr/schedule', 'POST', value);
        await refresh();
      }}
      onRevert={(value) => {
        schedule = value;
      }}
    >
      <Switch bind:checked={schedule.paused} size="sm"
        >Pause automatic sync</Switch
      >
      <FormField
        >Daily sync hour ({snapshot.timezone})<select
          class={formControlClass}
          bind:value={schedule.hour}
        >
          {#each Array.from({ length: 24 }, (_, i) => i) as hour (hour)}<option
              value={hour}>{String(hour).padStart(2, '0')}:00</option
            >{/each}
        </select></FormField
      >
    </AutoSaveForm>
  {/if}
</section>
