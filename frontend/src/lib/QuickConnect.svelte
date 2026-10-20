<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import { cardBlockClass, formControlClass } from './ui/styles';
  import { api } from './api';
  import { withVerification } from './authentication';
  import Button from './ui/Button.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import { twMerge } from 'tailwind-merge';
  import { reveal } from './ui/reveal';
  let { username } = $props<{ username: string }>();
  const ids = $props.id();
  let code = $state('');
  let device = $state<{
    device: string;
    client: string;
    version: string;
    confirmation: string;
  } | null>(null);
  let error = $state(''),
    busy = $state(false),
    approved = $state(false);
  async function inspect() {
    busy = true;
    error = '';
    approved = false;
    device = null;
    try {
      device = await api('/auth/quick-connect/inspect', 'POST', { code });
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function approve() {
    if (!device) return;
    const confirmation = device.confirmation;
    busy = true;
    error = '';
    try {
      const done = await withVerification(async () => {
        await api('/auth/quick-connect/approve', 'POST', {
          code,
          confirmation,
        });
      });
      // A cancelled verification approved nothing.
      if (!done) return;
      approved = true;
      device = null;
      code = '';
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<SettingsCard
  aria-label="Quick Connect"
  title="Connect a TV"
  description={`Enter the Quick Connect code displayed by your media client to sign it in as ${username}.`}
>
  <SettingRow label="TV code" for={`${ids}-code`}>
    <form
      class="flex flex-wrap items-center gap-2"
      onsubmit={(event) => {
        event.preventDefault();
        void inspect();
      }}
    >
      <input
        id={`${ids}-code`}
        class={twMerge(formControlClass, 'w-32 tracking-[0.2em]')}
        bind:value={code}
        oninput={() => {
          device = null;
          approved = false;
        }}
        pattern={'[0-9]{6}'}
        maxlength="6"
        inputmode="numeric"
        autocomplete="off"
        placeholder="000000"
        required
      />
      <Button type="submit" variant="secondary" size="form" disabled={busy}
        >Find device</Button
      >
    </form>
  </SettingRow>
  {#if device || error || approved}
    <div class="{cardBlockClass} grid gap-3" transition:reveal>
      {#if device}<p class="m-0 text-xs">
          Sign in to <strong>{device.device}</strong> using {device.client}
          {device.version} as <strong>{username}</strong>.
        </p>
        <div>
          <Button size="form" disabled={busy} onclick={() => void approve()}
            >Connect this device</Button
          >
        </div>{/if}
      {#if error}<Notice role="alert" variant="error" class="mb-0"
          >{error}</Notice
        >{/if}
      {#if approved}<p role="status" class="m-0 text-xs text-muted">
          Device approved. Continue on your TV.
        </p>{/if}
    </div>
  {/if}
</SettingsCard>
