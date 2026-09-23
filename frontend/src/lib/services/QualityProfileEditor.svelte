<script lang="ts">
  import { ArrowUp, ArrowDown, Plus, X } from '@lucide/svelte';
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import FormField from '../ui/FormField.svelte';
  import Switch from '../ui/Switch.svelte';
  import Notice from '../ui/Notice.svelte';
  import { formControlClass } from '../ui/styles';
  let { serviceId, created } = $props<{
    serviceId: string;
    created: () => Promise<void>;
  }>();
  type Quality = { id: number; name: string; resolution: number };
  let expanded = $state(false),
    loading = $state(false),
    busy = $state(false),
    error = $state(''),
    name = $state(''),
    qualities = $state<Quality[]>([]),
    selected = $state<number[]>([]),
    cutoff = $state(0),
    upgrades = $state(true);
  async function start() {
    expanded = true;
    loading = true;
    error = '';
    try {
      qualities = (
        await api<{ qualities: Quality[] }>(
          `/admin/managers/${serviceId}/quality-profiles`,
        )
      ).qualities;
      selected = qualities
        .filter(
          (q) =>
            [1080, 2160].includes(q.resolution) &&
            !['Raw-HD', 'BR-DISK'].includes(q.name),
        )
        .sort(
          (a, b) =>
            a.resolution - b.resolution ||
            sourceRank(a.name) - sourceRank(b.name),
        )
        .map((q) => q.id);
      cutoff = selected.at(-1) ?? 0;
    } catch (caught) {
      error = String(caught);
    } finally {
      loading = false;
    }
  }
  function sourceRank(name: string) {
    const value = name.toLowerCase();
    return value.includes('remux')
      ? 4
      : value.includes('bluray')
        ? 3
        : value.includes('webdl')
          ? 2
          : value.includes('webrip')
            ? 1
            : 0;
  }
  function toggle(id: number, enabled: boolean) {
    selected = enabled ? [...selected, id] : selected.filter((q) => q !== id);
    if (!selected.includes(cutoff)) cutoff = selected.at(-1) ?? 0;
  }
  function move(index: number, offset: number) {
    const next = [...selected];
    [next[index], next[index + offset]] = [next[index + offset], next[index]];
    selected = next;
  }
  async function create() {
    busy = true;
    error = '';
    try {
      await api(`/admin/managers/${serviceId}/quality-profiles`, 'POST', {
        name,
        qualities: selected,
        cutoff,
        upgrades,
      });
      await created();
      expanded = false;
      name = '';
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
</script>

{#if !expanded}<Button
    variant="secondary"
    size="sm"
    onclick={() => void start()}
    ><Plus size={14} /> Create quality profile</Button
  >
{:else}<form
    class="mt-4 grid gap-4 border border-line p-4"
    aria-label="New quality profile"
    onsubmit={(event) => {
      event.preventDefault();
      void create();
    }}
  >
    <div class="flex items-center justify-between">
      <h4 class="text-xs font-semibold">New quality profile</h4>
      <Button
        variant="ghost"
        size="sm"
        disabled={busy}
        aria-label="Close profile editor"
        onclick={() => (expanded = false)}><X size={15} /></Button
      >
    </div>
    {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
    {#if loading}<p role="status" class="text-xs text-muted">
        Loading qualities…
      </p>{:else}
      <FormField
        >Name<input
          class={formControlClass}
          bind:value={name}
          required
          maxlength="100"
          placeholder="Profile name"
        /></FormField
      >
      <fieldset class="grid grid-cols-2 gap-2 compact:grid-cols-1">
        <legend class="mb-2 text-xs text-muted">Allowed qualities</legend
        >{#each qualities as quality (quality.id)}<label
            class="flex items-center gap-2 text-xs"
            ><input
              type="checkbox"
              class="accent-accent"
              checked={selected.includes(quality.id)}
              onchange={(event) =>
                toggle(quality.id, event.currentTarget.checked)}
            />{quality.name}</label
          >{/each}
      </fieldset>
      {#if selected.length}<div>
          <p class="mb-2 text-xs text-muted">
            Quality order · lowest to highest
          </p>
          <ol class="grid gap-1">
            {#each selected as id, index (id)}<li
                class="flex items-center gap-2 border border-line px-2 py-1 text-xs"
              >
                <span class="w-5 text-muted">{index + 1}</span><span
                  class="flex-1"
                  >{qualities.find((q) => q.id === id)?.name}</span
                ><Button
                  variant="ghost"
                  size="sm"
                  aria-label={`Lower priority for ${qualities.find((q) => q.id === id)?.name}`}
                  disabled={index === 0}
                  onclick={() => move(index, -1)}><ArrowUp size={14} /></Button
                ><Button
                  variant="ghost"
                  size="sm"
                  aria-label={`Higher priority for ${qualities.find((q) => q.id === id)?.name}`}
                  disabled={index === selected.length - 1}
                  onclick={() => move(index, 1)}><ArrowDown size={14} /></Button
                >
              </li>{/each}
          </ol>
        </div>{/if}
      <Switch bind:checked={upgrades} size="sm"
        >Upgrade existing downloads</Switch
      >
      {#if upgrades}<FormField
          >Upgrade until<select
            class={formControlClass}
            bind:value={cutoff}
            required
            >{#each selected as id (id)}<option value={id}
                >{qualities.find((q) => q.id === id)?.name}</option
              >{/each}</select
          ></FormField
        >{/if}
      <Button
        type="submit"
        size="form"
        disabled={busy || !selected.length || !name.trim()}
        >{busy ? 'Creating…' : 'Create and use profile'}</Button
      >
    {/if}
  </form>{/if}
