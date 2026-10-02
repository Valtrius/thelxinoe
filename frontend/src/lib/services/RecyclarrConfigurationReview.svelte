<script lang="ts">
  import { untrack, onMount } from 'svelte';
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import Modal from '../ui/Modal.svelte';
  import YamlDiff from '../ui/YamlDiff.svelte';
  import { formControlClass } from '../ui/styles';
  import {
    mergeDefaults,
    type RecyclarrConfiguration,
    type YamlFiles,
  } from './recyclarr-configuration';
  let {
    configuration,
    candidate = false,
    provisionId,
    onClose,
    onUpdate,
    onEdit,
  } = $props<{
    configuration: RecyclarrConfiguration;
    candidate?: boolean;
    provisionId: string;
    onClose: () => void;
    onUpdate: (value: RecyclarrConfiguration) => void;
    onEdit: (files: YamlFiles, candidate: boolean) => void;
  }>();
  let snapshot = $state(untrack(() => configuration));
  let file = $state('recyclarr.yml'),
    error = $state(''),
    busy = $state(false);
  let merging = $state(false),
    merged = $state<YamlFiles | null>(null),
    conflicts = $state<string[]>([]);
  let alive = true;
  const incoming = $derived(
    candidate ? snapshot.candidate!.files : snapshot.defaults.files,
  );
  const after = $derived(merged ?? incoming);
  const names = $derived(
    [
      ...new Set([...Object.keys(snapshot.files), ...Object.keys(after)]),
    ].sort(),
  );
  const changed = $derived(
    names.filter((name) => snapshot.files[name] !== after[name]),
  );
  onMount(() => () => {
    alive = false;
  });
  async function restore() {
    busy = true;
    error = '';
    try {
      onUpdate(
        await api<RecyclarrConfiguration>(
          '/admin/recyclarr/configuration/defaults',
          'POST',
          { revision: snapshot.revision },
        ),
      );
      onClose();
    } catch (caught) {
      if (alive) error = String(caught);
    } finally {
      if (alive) busy = false;
    }
  }
  async function suggestions() {
    merging = true;
    try {
      const result = await mergeDefaults(
        snapshot.base_defaults.files,
        snapshot.files,
        snapshot.defaults.files,
      );
      if (alive) {
        merged = result.files;
        conflicts = result.conflicts;
      }
    } catch (caught) {
      if (alive) error = String(caught);
    } finally {
      if (alive) merging = false;
    }
  }
  async function update(action: 'preflight' | 'activate') {
    busy = true;
    error = '';
    try {
      const queued = await api<{ id: string }>(
        action === 'preflight'
          ? `/admin/service-updates/preflight/${provisionId}`
          : `/admin/service-updates/${snapshot.candidate!.operation_id}/activate`,
        'POST',
        {},
      );
      const id =
        action === 'activate' ? snapshot.candidate!.operation_id : queued.id;
      const deadline = Date.now() + 15 * 60 * 1000;
      while (alive && Date.now() < deadline) {
        const result = await api<{
          items: { id: string; state: string; error: string | null }[];
        }>('/admin/service-updates');
        const current = result.items.find((item) => item.id === id);
        if (
          current &&
          [
            'ready',
            'committed',
            'blocked',
            'rolled-back',
            'runtime-failure',
            'recovery-required',
          ].includes(current.state)
        ) {
          const next = await api<RecyclarrConfiguration>(
            '/admin/recyclarr/configuration',
          );
          snapshot = next;
          onUpdate(next);
          if (current.state === 'committed') onClose();
          if (current.state !== 'ready' && current.state !== 'committed')
            error =
              current.error ??
              'Candidate validation failed. Review its diagnostics.';
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, 1500));
      }
      if (alive)
        error = 'The update is still running. Check its status in Updates.';
    } catch (caught) {
      if (alive) error = String(caught);
    } finally {
      if (alive) busy = false;
    }
  }
  function resolveConflict(useDefault: boolean) {
    if (!merged) return;
    if (useDefault) {
      const next = { ...merged };
      if (incoming[file] === undefined) delete next[file];
      else next[file] = incoming[file];
      merged = next;
    }
    conflicts = conflicts.filter((name) => name !== file);
  }
</script>

<Modal
  title={candidate ? 'Review candidate configuration' : 'Review defaults'}
  size="editor"
  {onClose}
>
  <div class="shrink-0 space-y-2 border-b border-line px-4 py-3 text-xs">
    <p>
      {candidate
        ? snapshot.candidate?.valid
          ? 'Candidate configuration is compatible.'
          : 'Candidate configuration needs validation.'
        : 'Use defaults replaces the whole file set and follows defaults on future upgrades.'}
    </p>
    <p class="text-muted">
      {changed.length}
      {changed.length === 1 ? 'file changes' : 'files change'}{#if candidate}
        · Active files stay in use until the upgrade.{/if}
    </p>
    {#if !candidate && snapshot.mode === 'customized'}<Button
        size="sm"
        variant="secondary"
        loading={merging}
        onclick={() => void suggestions()}>Review default suggestions</Button
      >{/if}
    {#if error}<p class="text-danger" role="alert">{error}</p>{/if}
  </div>
  <div
    class="flex shrink-0 flex-wrap items-center gap-3 border-b border-line px-4 py-2"
  >
    <label class="flex min-w-0 flex-1 items-center gap-2 text-xs"
      >File<select class={formControlClass} bind:value={file}
        >{#each names as name (name)}<option value={name}
            >{name}{snapshot.files[name] === undefined
              ? ' — added'
              : after[name] === undefined
                ? ' — removed'
                : snapshot.files[name] !== after[name]
                  ? ' — modified'
                  : ''}</option
          >{/each}</select
      ></label
    >
  </div>
  {#if merged && conflicts.includes(file)}<div
      class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-line px-4 py-2 text-xs text-warning"
    >
      <span>Both you and the defaults changed this file.</span>
      <div class="flex gap-2">
        <Button
          size="sm"
          variant="secondary"
          onclick={() => resolveConflict(false)}>Keep customized file</Button
        ><Button
          size="sm"
          variant="secondary"
          onclick={() => resolveConflict(true)}>Use new default file</Button
        >
      </div>
    </div>{/if}
  <div class="min-h-0 flex-1">
    <YamlDiff
      before={snapshot.files[file] ?? ''}
      after={after[file] ?? ''}
      afterLabel={merged
        ? 'Merged draft'
        : candidate
          ? 'Candidate configuration'
          : 'Installed defaults'}
    />
  </div>
  {#if candidate && snapshot.candidate?.diagnostics.length}<details
      class="max-h-36 shrink-0 overflow-auto border-t border-line px-4 py-2 text-xs text-danger"
    >
      <summary>Candidate diagnostics</summary
      >{#each snapshot.candidate.diagnostics as problem, index (index)}<pre
          class="mt-2 whitespace-pre-wrap">{problem.file}:{problem.line} — {problem.message}</pre>{/each}
    </details>{/if}
  <footer
    class="flex shrink-0 flex-wrap justify-end gap-2 border-t border-line px-4 py-3"
  >
    <Button variant="secondary" onclick={onClose}>Close</Button>
    {#if candidate}<Button
        variant="secondary"
        disabled={busy}
        onclick={() => onEdit(incoming, true)}>Edit candidate YAML</Button
      ><Button
        variant="secondary"
        loading={busy}
        onclick={() => void update('preflight')}>Revalidate candidate</Button
      ><Button
        disabled={!snapshot.candidate?.valid || busy}
        onclick={() => void update('activate')}>Install upgrade</Button
      >
    {:else if merged}<Button
        disabled={conflicts.length > 0}
        onclick={() => onEdit(merged!, false)}>Edit merged YAML</Button
      >
    {:else}<Button loading={busy} onclick={() => void restore()}
        >Use defaults</Button
      >{/if}
  </footer>
</Modal>
