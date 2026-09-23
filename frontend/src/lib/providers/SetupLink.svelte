<script lang="ts">
  import type { Snippet } from 'svelte';
  import { desktop } from '../api';
  import { api } from './api';

  let { href, children }: { href: string; children: Snippet } = $props();
  let error = $state('');
</script>

<a
  {href}
  target="_blank"
  rel="noopener noreferrer"
  class="text-accent underline underline-offset-4"
  onclick={(event) => {
    if (!desktop) return;
    event.preventDefault();
    error = '';
    void api.openExternal(href).catch(() => {
      error = `Could not open your browser. Open ${href} manually.`;
    });
  }}>{@render children()}</a
>
{#if error}<span role="alert" class="block break-all">{error}</span>{/if}
