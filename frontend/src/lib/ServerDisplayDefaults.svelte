<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { api } from './api';
  import { LatestRequest } from './latest-request';
  import { captureSession } from './session';
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
    refreshPending = $state(false);
  const dirty = $derived(
    confirmed && JSON.stringify(draft) !== JSON.stringify(confirmed),
  );
  const requests = new LatestRequest();
  const ownsSession = captureSession();
  let active = true;
  onDestroy(() => {
    active = false;
    requests.invalidate();
  });
  $effect(() => {
    void revision;
    untrack(() => void load());
  });
  $effect(() => {
    if (!saving && !dirty && refreshPending) untrack(() => void load());
  });
  async function load() {
    refreshPending = saving || Boolean(dirty);
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
      ) {
        refreshPending = true;
        return;
      }
      refreshPending = false;
      confirmed = { timezone: value.timezone, time_format: value.time_format };
      draft = { ...confirmed };
    } catch (caught) {
      if (current()) error = String(caught);
    }
  }
  async function save(submitted: Defaults) {
    if (!active || !ownsSession()) return;
    requests.invalidate();
    const value = await api<Defaults>('/admin/settings', 'PUT', submitted);
    if (!active || !ownsSession()) return;
    requests.invalidate();
    confirmed = { ...submitted };
    changed(value.timezone);
  }
</script>

<Panel>
  <h2>Display defaults</h2>
  {#if confirmed}
    <AutoSaveForm
      label="Server display defaults"
      class={inlineFormClass}
      value={draft}
      baseline={confirmed}
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
  {:else if !error}<p role="status">Loading display defaults…</p>{/if}
  {#if error}<Notice role="alert" variant="error">{error}</Notice>
    <Button variant="secondary" size="form" onclick={() => void load()}
      >Retry display defaults</Button
    >{/if}
</Panel>
