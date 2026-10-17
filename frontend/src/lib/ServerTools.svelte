<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import ServerToolRow from './ServerToolRow.svelte';
  import type { ServerToolStatus } from './server-tools';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import Button from './ui/Button.svelte';
  import { cardBlockClass, cardTableClass } from './ui/styles';
  let status = $state<ServerToolStatus | null>(null);
  let error = $state('');
  let checking = $state(false);
  let alive = true;
  let sequence = 0;
  async function refresh() {
    const current = ++sequence;
    try {
      const value = await api<ServerToolStatus>('/admin/tools');
      if (alive && current === sequence) {
        status = value;
        error = '';
      }
    } catch (caught) {
      if (alive && current === sequence) error = String(caught);
    }
  }
  async function check() {
    checking = true;
    try {
      await api('/admin/tools/check', 'POST', {});
      await refresh();
    } catch (caught) {
      error = String(caught);
    } finally {
      checking = false;
    }
  }
  onMount(() => {
    const update = () => void refresh();
    update();
    window.addEventListener('thelxinoe-tools', update);
    const timer = setInterval(update, 15000);
    return () => {
      alive = false;
      clearInterval(timer);
      window.removeEventListener('thelxinoe-tools', update);
    };
  });
</script>

<SettingsCard
  id="server-tools"
  aria-label="Server tools"
  title="Tools"
  description="Bundled binaries used for downloads, streaming and media analysis."
>
  {#snippet actions()}
    <Button
      variant="secondary"
      size="form"
      disabled={checking || status?.supported === false}
      onclick={check}>Check for updates</Button
    >
  {/snippet}
  {#if status?.supported === false || error}
    <div class="{cardBlockClass} grid gap-2">
      {#if status?.supported === false}<p class="m-0 text-xs text-muted">
          Managed tools require the Linux x86-64 server image.
        </p>{/if}
      {#if error}<p role="alert" class="m-0 text-xs text-danger">
          {error}
        </p>{/if}
    </div>
  {/if}
  <div class="overflow-x-auto">
    <table class={cardTableClass} aria-label="Server tool versions">
      <thead
        ><tr
          ><th>Tool</th><th>Installed</th><th>Pinned</th><th>Updates</th><th
            class="text-right">Status</th
          ></tr
        ></thead
      >
      <tbody
        >{#if !status && !error}<tr
            ><td colspan="5"
              ><ContentSkeleton label="Loading server tools" count={3} /></td
            ></tr
          >{/if}{#each status?.items ?? [] as item (item.id)}<ServerToolRow
            {item}
            supported={status?.supported ?? false}
            {refresh}
          />{/each}</tbody
      >
    </table>
  </div>
</SettingsCard>
