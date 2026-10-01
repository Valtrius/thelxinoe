<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import ServerToolRow from './ServerToolRow.svelte';
  import type { ServerToolStatus } from './server-tools';
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

<section
  aria-label="Server tools"
  class="mt-5 min-w-0 border-t border-line pt-4"
>
  <div class="flex items-center justify-between gap-3">
    <h3 class="font-semibold">Tools</h3>
    <button
      class="border border-line px-3 py-1.5 text-xs disabled:opacity-40"
      disabled={checking || status?.supported === false}
      onclick={check}>Check for updates</button
    >
  </div>
  {#if status?.supported === false}<p class="mt-2 text-xs text-muted">
      Managed tools require the Linux x86-64 server image.
    </p>{/if}
  {#if error}<p role="alert" class="mt-2 text-xs text-danger">{error}</p>{/if}
  <div class="mt-2 overflow-x-auto">
    <table class="w-full text-left" aria-label="Server tool versions">
      <thead class="text-xs text-muted"
        ><tr
          ><th class="py-2 pr-3 font-normal">Tool</th><th
            class="py-2 pr-3 font-normal">Installed</th
          ><th class="py-2 pr-3 font-normal">Pinned</th><th
            class="py-2 pr-3 font-normal">Updates</th
          ><th class="py-2 font-normal">Status</th></tr
        ></thead
      >
      <tbody
        >{#each status?.items ?? [] as item (item.id)}<ServerToolRow
            {item}
            supported={status?.supported ?? false}
            {refresh}
          />{/each}</tbody
      >
    </table>
  </div>
</section>
