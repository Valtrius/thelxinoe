<script lang="ts">
  import { Plus } from '@lucide/svelte';
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import FormField from '../ui/FormField.svelte';
  import Switch from '../ui/Switch.svelte';
  import Notice from '../ui/Notice.svelte';
  import Modal from '../ui/Modal.svelte';
  import SortableToggleList from '../ui/SortableToggleList.svelte';
  import { formControlClass } from '../ui/styles';
  let { serviceId, created } = $props<{
    serviceId: string;
    created: () => Promise<void>;
  }>();
  type Quality = { id: number; name: string; resolution?: number };
  let expanded = $state(false),
    loading = $state(false),
    busy = $state(false),
    error = $state(''),
    name = $state(''),
    qualities = $state<Quality[]>([]),
    enabled = $state<number[]>([]),
    cutoff = $state(0),
    upgrades = $state(true);
  const selected = $derived(
    qualities
      .filter((quality) => enabled.includes(quality.id))
      .map((quality) => quality.id),
  );
  async function start() {
    name = '';
    upgrades = true;
    expanded = true;
    loading = true;
    error = '';
    try {
      qualities = (
        await api<{ qualities: Quality[] }>(
          `/admin/managers/${serviceId}/quality-profiles`,
        )
      ).qualities.reverse();
      enabled = qualities
        .filter(
          (q) =>
            [1080, 2160].includes(q.resolution ?? 0) &&
            !['Raw-HD', 'BR-DISK'].includes(q.name),
        )
        .map((q) => q.id);
      cutoff = enabled[0] ?? 0;
    } catch (caught) {
      error = String(caught);
    } finally {
      loading = false;
    }
  }
  function toggle(id: number, checked: boolean) {
    enabled = checked ? [...enabled, id] : enabled.filter((q) => q !== id);
    if (!enabled.includes(cutoff))
      cutoff = qualities.find((q) => enabled.includes(q.id))?.id ?? 0;
  }
  async function create() {
    busy = true;
    error = '';
    try {
      await api(`/admin/managers/${serviceId}/quality-profiles`, 'POST', {
        name,
        // Servarr stores profiles from lowest to highest priority.
        qualities: [...selected].reverse(),
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

<Button variant="secondary" size="sm" onclick={() => void start()}
  ><Plus size={14} /> Create quality profile</Button
>
{#if expanded}<Modal
    title="New quality profile"
    {busy}
    onClose={() => (expanded = false)}
  >
    <form
      class="grid gap-4 [&_label]:m-0"
      aria-label="New quality profile"
      onsubmit={(event) => {
        event.preventDefault();
        void create();
      }}
    >
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
        <div>
          <p class="mb-2 text-xs text-muted">
            Qualities (highest priority first)
          </p>
          <SortableToggleList
            bind:items={qualities}
            {enabled}
            label="Quality priority"
            onToggle={toggle}
          />
        </div>
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
    </form></Modal
  >{/if}
