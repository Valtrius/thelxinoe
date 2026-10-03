<script lang="ts">
  import type { Snippet } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { desktop } from '../api';
  import { api } from './api';

  let {
    href,
    publicPage,
    children,
  }: {
    href: string;
    publicPage?: 'home' | 'privacy';
    children: Snippet;
  } = $props();
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
    const opening = publicPage
      ? invoke('open_public_page', { page: publicPage })
      : api.openExternal(href);
    void opening.catch(() => {
      error = `Could not open your browser. Open ${href} manually.`;
    });
  }}>{@render children()}</a
>
{#if error}<span role="alert" class="block break-all">{error}</span>{/if}
