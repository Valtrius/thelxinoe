<script lang="ts">
  import { api, type User } from './api';
  import TimezoneSelect from './TimezoneSelect.svelte';
  import { onDestroy } from 'svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import Panel from './ui/Panel.svelte';
  import { errorClass, inlineFormClass } from './ui/styles';
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
  };
  const userId = $derived(user.id);
  let timezone = $state(''),
    serverTimezone = $state('UTC'),
    timeFormat = $state<'12h' | '24h'>('24h'),
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
        timeFormat = value.time_format;
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
  async function save() {
    const value = await api<Preferences>('/me/preferences', 'PUT', {
      timezone: timezone || null,
      time_format: timeFormat,
    });
    if (!active) return;
    timezone = value.timezone_override ?? '';
    serverTimezone = value.server_timezone;
    timeFormat = value.time_format;
    changed(value.timezone, value.time_format);
  }
</script>

<Panel>
  <h2>Your display preferences</h2>
  <AutoSaveForm
    label="Display preferences"
    class={inlineFormClass}
    onsave={save}
    disabled={busy || Boolean(error)}
  >
    <TimezoneSelect
      label="Display timezone"
      bind:value={timezone}
      defaultTimezone={serverTimezone}
      disabled={busy}
    />
    <label
      >Time format<select bind:value={timeFormat} disabled={busy}
        ><option value="24h">24-hour</option><option value="12h">12-hour</option
        ></select
      ></label
    >
  </AutoSaveForm>
  <p class="text-muted">
    Use the server default or choose your own timezone and time format. Regional
    timezones adjust automatically for daylight saving time.
  </p>
  {#if error}<p role="alert" class={errorClass}>{error}</p>{/if}
</Panel>
