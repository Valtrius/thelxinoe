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
    changed: (zone: string) => void;
    revision?: number;
  }>();
  type Preferences = {
    timezone: string;
    timezone_override: string | null;
    server_timezone: string;
  };
  const userId = $derived(user.id);
  let timezone = $state(''),
    serverTimezone = $state('UTC'),
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
        changed(value.timezone);
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
    });
    if (!active) return;
    timezone = value.timezone_override ?? '';
    serverTimezone = value.server_timezone;
    changed(value.timezone);
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
  </AutoSaveForm>
  <p class="text-muted">
    Use the server default or choose your own timezone. Regional timezones
    adjust automatically for daylight saving time.
  </p>
  {#if error}<p role="alert" class={errorClass}>{error}</p>{/if}
</Panel>
