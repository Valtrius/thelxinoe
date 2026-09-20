<script lang="ts">
  import { api, type User } from './api';
  import TimezoneSelect from './TimezoneSelect.svelte';
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
    error = $state(''),
    saved = $state(false);
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
    error = '';
    saved = false;
    busy = true;
    try {
      const value = await api<Preferences>('/me/preferences', 'PUT', {
        timezone: timezone || null,
      });
      timezone = value.timezone_override ?? '';
      serverTimezone = value.server_timezone;
      changed(value.timezone);
      saved = true;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="panel">
  <h2>Your display preferences</h2>
  <form
    class="inline-form"
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
  >
    <TimezoneSelect
      label="Display timezone"
      bind:value={timezone}
      defaultTimezone={serverTimezone}
      disabled={busy}
    />
    <button class="primary" disabled={busy}>Save display preferences</button>
  </form>
  <p class="muted">
    Use the server default or choose your own timezone. Regional timezones
    adjust automatically for daylight saving time.
  </p>
  {#if error}<p role="alert" class="error">{error}</p>{/if}{#if saved}<p
      role="status"
    >
      Display preferences saved.
    </p>{/if}
</section>
