<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { api } from './api';
  import { LatestRequest } from './latest-request';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import FormField from './ui/FormField.svelte';
  import Notice from './ui/Notice.svelte';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import TimezoneSelect from './TimezoneSelect.svelte';
  import { formControlClass, inlineFormClass } from './ui/styles';

  let { revision = 0, changed } = $props<{
    revision?: number;
    changed: (timezone: string) => void;
  }>();
  type Defaults = { timezone: string; time_format: '12h' | '24h' };
  let confirmed = $state<Defaults | null>(null);
  let draft = $state<Defaults>({ timezone: 'UTC', time_format: '24h' });
  let error = $state(''),
    saving = $state(false),
    baseline = $state(0);
  const requests = new LatestRequest();
  onDestroy(() => requests.invalidate());
  $effect(() => {
    void revision;
    untrack(() => void load());
  });
  async function load() {
    const current = requests.begin();
    const previous = JSON.stringify(draft);
    error = '';
    try {
      const value = await api<Defaults>('/admin/settings');
      if (!current()) return;
      changed(value.timezone);
      if (
        confirmed &&
        (saving ||
          JSON.stringify(draft) !== previous ||
          previous !== JSON.stringify(confirmed))
      )
        return;
      confirmed = value;
      draft = { ...value };
      baseline++;
    } catch (caught) {
      if (current()) error = String(caught);
    }
  }
  async function save(submitted: Defaults) {
    const current = requests.begin();
    const value = await api<Defaults>('/admin/settings', 'PUT', submitted);
    if (!current()) return;
    confirmed = { ...submitted };
    changed(value.timezone);
  }
</script>

<Panel>
  <h2>Display defaults</h2>
  {#if confirmed}
    {#key baseline}
      <AutoSaveForm
        label="Server display defaults"
        class={inlineFormClass}
        value={draft}
        onsave={save}
        bind:busy={saving}
        onRevert={(previous) => (draft = previous)}
      >
        <TimezoneSelect
          label="Server default timezone"
          bind:value={draft.timezone}
        />
        <FormField
          >Server default time format<select
            class={formControlClass}
            bind:value={draft.time_format}
          >
            <option value="24h">24-hour</option><option value="12h"
              >12-hour</option
            >
          </select></FormField
        >
      </AutoSaveForm>
    {/key}
  {:else if !error}<p role="status">Loading display defaults…</p>{/if}
  {#if error}<Notice role="alert" variant="error">{error}</Notice>
    <Button variant="secondary" size="form" onclick={() => void load()}
      >Retry display defaults</Button
    >{/if}
</Panel>
