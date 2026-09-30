<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../api';
  import AutoSaveForm from '../ui/AutoSaveForm.svelte';
  import FormField from '../ui/FormField.svelte';
  import Switch from '../ui/Switch.svelte';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import { formControlClass } from '../ui/styles';
  import RecyclarrTarget from './RecyclarrTarget.svelte';
  import type { Guide, Target } from './recyclarr';
  type Run = {
    id: string;
    state: string;
    created_at: number;
    updated_at: number;
    error: string | null;
  };
  type Snapshot = {
    settings: { paused: boolean; hour: number; next_run: number } | null;
    targets: Target[];
    runs: Run[];
    timezone: string;
  };
  let snapshot = $state<Snapshot | null>(null);
  let catalogs = $state<Record<string, Guide[]>>({});
  let evidence = $state<
    Record<string, { revision: string; value?: unknown; error?: string }>
  >({});
  let error = $state('');
  let feedback = $state('');
  let feedbackRun = $state('');
  let schedule = $state({ paused: false, hour: 4 });
  let loaded = $state(false);
  async function refresh() {
    try {
      snapshot = await api<Snapshot>('/admin/recyclarr');
      for (const run of snapshot.runs) {
        if (
          evidence[run.id] &&
          evidence[run.id].revision !== `${run.updated_at}:${run.state}`
        )
          void loadEvidence(run);
      }
      if (!loaded && snapshot.settings) {
        schedule = {
          paused: snapshot.settings.paused,
          hour: snapshot.settings.hour,
        };
        loaded = true;
      }
      if (
        snapshot.runs.some(
          (r) =>
            r.id === feedbackRun &&
            !['queued', 'running', 'retrying'].includes(r.state),
        )
      ) {
        feedback = '';
        feedbackRun = '';
      }
      error = '';
    } catch (failure) {
      error = String(failure);
    }
  }
  async function loadEvidence(run: Run) {
    const revision = `${run.updated_at}:${run.state}`;
    if (evidence[run.id]?.revision === revision && !evidence[run.id]?.error)
      return;
    evidence[run.id] = { revision };
    try {
      const detail = await api<{ evidence: unknown }>(
        `/admin/recyclarr/runs/${run.id}`,
      );
      if (evidence[run.id]?.revision === revision)
        evidence[run.id] = { revision, value: detail.evidence };
    } catch (failure) {
      if (evidence[run.id]?.revision === revision)
        evidence[run.id] = { revision, error: String(failure) };
    }
  }
  async function refreshCatalogs() {
    try {
      for (const kind of ['radarr', 'sonarr'])
        catalogs[kind] = (
          await api<{ items: Guide[] }>(`/admin/recyclarr/catalog/${kind}`)
        ).items;
      error = '';
    } catch (failure) {
      error = String(failure);
    }
  }
  async function run(preview = false) {
    try {
      const queued = await api<{ id: string }>(
        `/admin/recyclarr/${preview ? 'preview' : 'sync'}`,
        'POST',
        {},
      );
      feedbackRun = queued.id;
      feedback = preview ? 'Preview queued' : 'Sync queued';
      await refresh();
    } catch (failure) {
      error = String(failure);
    }
  }
  onMount(() => {
    void refresh().then(refreshCatalogs);
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
    <div class="flex flex-wrap gap-2">
      <Button
        size="form"
        variant="secondary"
        onclick={() => void refreshCatalogs()}>Refresh profiles</Button
      >
      <Button size="form" variant="secondary" onclick={() => void run(true)}
        >Preview</Button
      >
      <Button size="form" onclick={() => void run()}>Sync now</Button>
    </div>
  </div>
  <p class="text-[11px] leading-5 text-muted">
    Guide changes apply automatically to custom formats and quality profiles.
    Defaults apply to new additions. Existing movies and series keep their
    assigned profile.
  </p>
  {#if error}<Notice tone="danger" role="alert">{error}</Notice>{/if}
  {#if feedback}<span class="text-[10px] text-muted" role="status"
      >{feedback}</span
    >{/if}
  {#if snapshot?.runs[0]}<span class="text-[10px] text-muted"
      >Latest run: {snapshot.runs[0].state} &middot; {new Date(
        snapshot.runs[0].created_at * 1000,
      ).toLocaleString()}</span
    >{/if}
  {#if snapshot?.settings && loaded}
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
          >{#each Array.from({ length: 24 }, (_, i) => i) as hour (hour)}<option
              value={hour}>{String(hour).padStart(2, '0')}:00</option
            >{/each}</select
        ></FormField
      >
      <span class="text-[10px] text-muted"
        >Next sync: {schedule.paused
          ? 'Paused'
          : new Date(snapshot.settings.next_run * 1000).toLocaleString()}</span
      >
    </AutoSaveForm>
  {/if}
  {#each snapshot?.targets ?? [] as target (target.service_id)}
    <RecyclarrTarget
      {target}
      guides={catalogs[target.kind] ?? []}
      changed={refresh}
    />
  {:else}
    <p class="text-[11px] text-muted">
      Connect Radarr or Sonarr to apply guide defaults.
    </p>
  {/each}
  <details class="border-t border-line pt-3 text-[11px]">
    <summary class="cursor-pointer">Sync history</summary>
    {#each snapshot?.runs ?? [] as run (run.id)}
      <details
        class="border-b border-line py-3"
        ontoggle={(event) => {
          if (event.currentTarget.open) void loadEvidence(run);
        }}
      >
        <summary class="cursor-pointer"
          >{new Date(run.created_at * 1000).toLocaleString()} · {run.state}</summary
        >
        {#if run.error}<Notice tone="danger">{run.error}</Notice>{/if}
        {#if evidence[run.id]?.error}<Notice tone="danger"
            >{evidence[run.id].error}</Notice
          >{/if}
        <pre
          class="mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-all text-[10px] text-muted">{JSON.stringify(
            evidence[run.id]?.value ?? 'Loading evidence…',
            null,
            2,
          )}</pre>
      </details>
    {:else}<p class="py-3 text-muted">No syncs yet.</p>{/each}
  </details>
</section>
