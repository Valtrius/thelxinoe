<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { api, type User } from './api';
  import TimezoneSelect from './TimezoneSelect.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { captureSession } from './session';
  import { LatestRequest } from './latest-request';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import Panel from './ui/Panel.svelte';
  import ProfilePicture from './ProfilePicture.svelte';
  import AppearanceSettings from './AppearanceSettings.svelte';
  import { inlineFormClass } from './ui/styles';
  let {
    user,
    changed,
    avatarChanged,
    revision = 0,
  } = $props<{
    user: User;
    changed: (zone: string, timeFormat: '12h' | '24h') => void;
    avatarChanged: (id: string, avatar: string | null) => void;
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
  type Draft = { timezone: string; time_format: '' | '12h' | '24h' };
  const userId = $derived(user.id);
  let timezone = $state(''),
    serverTimezone = $state('UTC'),
    timeFormat = $state<'' | '12h' | '24h'>(''),
    serverTimeFormat = $state<'12h' | '24h'>('24h'),
    confirmed = $state<Draft | null>(null),
    saving = $state(false),
    refreshPending = $state(false),
    error = $state('');
  const dirty = $derived(
    confirmed &&
      (timezone !== confirmed.timezone || timeFormat !== confirmed.time_format),
  );
  const reads = new LatestRequest();
  let active = true;
  const ownsSession = captureSession();
  onDestroy(() => {
    active = false;
    reads.invalidate();
  });
  $effect(() => {
    void userId;
    void revision;
    untrack(() => void load());
  });
  $effect(() => {
    if (!saving && !dirty && refreshPending) untrack(() => void load());
  });
  async function load() {
    if (saving || dirty) {
      refreshPending = true;
      return;
    }
    refreshPending = false;
    const current = reads.begin();
    const previous = JSON.stringify({ timezone, time_format: timeFormat });
    error = '';
    try {
      const value = await api<Preferences>('/me/preferences');
      if (!current()) return;
      if (
        saving ||
        dirty ||
        JSON.stringify({ timezone, time_format: timeFormat }) !== previous
      ) {
        refreshPending = true;
        return;
      }
      timezone = value.timezone_override ?? '';
      serverTimezone = value.server_timezone;
      timeFormat = value.time_format_override ?? '';
      serverTimeFormat = value.server_time_format;
      confirmed = { timezone, time_format: timeFormat };
      changed(value.timezone, value.time_format);
    } catch (caught) {
      if (current()) error = String(caught);
    }
  }
  async function save(submitted: Draft) {
    if (!active || !ownsSession()) return;
    reads.invalidate();
    const value = await api<Preferences>('/me/preferences', 'PUT', {
      timezone: submitted.timezone || null,
      time_format: submitted.time_format || null,
    });
    if (!active || !ownsSession()) return;
    reads.invalidate();
    confirmed = { ...submitted };
    serverTimezone = value.server_timezone;
    serverTimeFormat = value.server_time_format;
    if (timezone === submitted.timezone && timeFormat === submitted.time_format)
      changed(value.timezone, value.time_format);
  }
</script>

<Panel aria-label="Your display preferences">
  <h2>Your display preferences</h2>
  <div class="grid gap-6">
    <ProfilePicture {user} changed={avatarChanged} />
    {#if confirmed}<AutoSaveForm
        label="Display preferences"
        class={inlineFormClass}
        onsave={save}
        bind:busy={saving}
        value={{ timezone, time_format: timeFormat }}
        baseline={confirmed}
        onRevert={(previous) => {
          timezone = previous.timezone;
          timeFormat = previous.time_format;
        }}
      >
        <TimezoneSelect
          label="Display timezone"
          bind:value={timezone}
          defaultTimezone={serverTimezone}
        />
        <FormField
          >Display time format<select
            class={formControlClass}
            bind:value={timeFormat}
            ><option value=""
              >Use server default ({serverTimeFormat === '12h'
                ? '12-hour'
                : '24-hour'})</option
            ><option value="24h">24-hour</option><option value="12h"
              >12-hour</option
            ></select
          ></FormField
        >
      </AutoSaveForm>{:else if !error}<p role="status">
        Loading display preferences…
      </p>{/if}
    {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
    <AppearanceSettings />
  </div>
</Panel>
