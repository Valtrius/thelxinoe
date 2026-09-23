<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { api, type User } from './api';
  import TimezoneSelect from './TimezoneSelect.svelte';
  import { onDestroy } from 'svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import Panel from './ui/Panel.svelte';
  import { inlineFormClass } from './ui/styles';
  let {
    user,
    changed,
    revision = 0,
  } = $props<{
    user: User;
    changed: (zone: string, timeFormat: '12h' | '24h') => void;
    revision?: number;
  }>();
  type Preferences = {
    timezone: string;
    timezone_override: string | null;
    server_timezone: string;
    time_format: '12h' | '24h';
    time_format_override: '12h' | '24h' | null;
    server_time_format: '12h' | '24h';
  };
  const userId = $derived(user.id);
  let timezone = $state(''),
    serverTimezone = $state('UTC'),
    timeFormat = $state<'' | '12h' | '24h'>(''),
    serverTimeFormat = $state<'12h' | '24h'>('24h'),
    busy = $state(true),
    error = $state('');
  let active = true;
  onDestroy(() => (active = false));
  $effect(() => {
    void userId;
    void revision;
    let active = true;
    busy = true;
    error = '';
    void api<Preferences>('/me/preferences')
      .then((value) => {
        if (!active) return;
        timezone = value.timezone_override ?? '';
        serverTimezone = value.server_timezone;
        timeFormat = value.time_format_override ?? '';
        serverTimeFormat = value.server_time_format;
        changed(value.timezone, value.time_format);
      })
      .catch((e) => {
        if (active) error = String(e);
      })
      .finally(() => {
        if (active) busy = false;
      });
    return () => {
      active = false;
    };
  });
  async function save(submitted: {
    timezone: string;
    time_format: '' | '12h' | '24h';
  }) {
    const value = await api<Preferences>('/me/preferences', 'PUT', {
      timezone: submitted.timezone || null,
      time_format: submitted.time_format || null,
    });
    if (!active) return;
    serverTimezone = value.server_timezone;
    serverTimeFormat = value.server_time_format;
    if (timezone === submitted.timezone && timeFormat === submitted.time_format)
      changed(value.timezone, value.time_format);
  }
</script>

<Panel>
  <h2>Your display preferences</h2>
  {#if !busy && !error}<AutoSaveForm
      label="Display preferences"
      class={inlineFormClass}
      onsave={save}
      value={{ timezone, time_format: timeFormat }}
      onRevert={(previous) => {
        timezone = previous.timezone;
        timeFormat = previous.time_format;
      }}
    >
      <TimezoneSelect
        label="Display timezone"
        bind:value={timezone}
        defaultTimezone={serverTimezone}
        disabled={busy}
      />
      <FormField
        >Display time format<select
          class={formControlClass}
          bind:value={timeFormat}
          disabled={busy}
          ><option value=""
            >Use server default ({serverTimeFormat === '12h'
              ? '12-hour'
              : '24-hour'})</option
          ><option value="24h">24-hour</option><option value="12h"
            >12-hour</option
          ></select
        ></FormField
      >
    </AutoSaveForm>{/if}
  {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
</Panel>
