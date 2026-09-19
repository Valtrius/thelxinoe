<script lang="ts">
  import { untrack } from 'svelte';
  import { api, type User } from './api';
  let { user, changed } = $props<{
    user: User;
    changed: (zone: string) => void;
  }>();
  let timezone = $state(''),
    error = $state(''),
    saved = $state(false);
  $effect(() => {
    const zone = user.timezone;
    untrack(() => (timezone = zone));
  });
  async function save() {
    error = '';
    saved = false;
    try {
      const value = await api<{ timezone: string }>('/me/preferences', 'PUT', {
        timezone,
      });
      changed(value.timezone);
      saved = true;
    } catch (e) {
      error = String(e);
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
    <label
      >Display timezone<input
        bind:value={timezone}
        required
        placeholder="Europe/Paris"
        list="timezones"
      /></label
    ><datalist id="timezones"
      ><option value="UTC"></option><option value="Europe/Paris"
      ></option><option value="America/New_York"></option><option
        value="Asia/Tokyo"
      ></option></datalist
    ><button class="primary">Save display preferences</button>
  </form>
  {#if error}<p role="alert" class="error">{error}</p>{/if}{#if saved}<p
      role="status"
    >
      Display preferences saved.
    </p>{/if}
</section>
