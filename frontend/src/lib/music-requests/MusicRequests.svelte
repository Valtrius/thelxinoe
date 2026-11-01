<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { Disc3 } from '@lucide/svelte';
  import { api, type User } from '../api';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import { requestLabel, type MusicRequest } from './types';
  let { user, loaded } = $props<{
    user: User;
    loaded: (count: number) => void;
  }>();
  let requests = $state<MusicRequest[]>([]),
    busy = $state<string | null>(null),
    error = $state('');
  let generation = 0,
    active = true;

  export async function load() {
    const version = ++generation;
    try {
      const result = await api<{ items: MusicRequest[] }>(
        '/acquisition/requests',
      );
      if (!active || version !== generation) return;
      requests = result.items;
      error = '';
      loaded(requests.length);
    } catch (caught) {
      if (active && version === generation) error = String(caught);
    }
  }
  async function act(request: MusicRequest, action: string) {
    busy = request.id;
    error = '';
    try {
      await api(`/acquisition/requests/${request.id}`, 'POST', { action });
      await load();
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = null;
    }
  }
  onMount(() => void load());
  onDestroy(() => {
    active = false;
  });
</script>

{#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
{#if requests.length}<section class="grid gap-3" aria-label="Music requests">
    <h3 class="mt-4 text-xs font-semibold text-muted">Music</h3>
    {#each requests as request (request.id)}<article
        class="flex items-center gap-4 border border-line bg-surface p-4 compact:flex-wrap"
      >
        <div class="flex min-w-0 flex-1 items-center gap-4">
          <span
            class="grid aspect-square w-13 shrink-0 place-items-center bg-surface-strong text-muted"
            ><Disc3 size={20} /></span
          ><span class="min-w-0"
            ><strong class="block text-sm">{request.title}</strong><span
              class="mt-1 block text-xs text-muted"
              >{requestLabel(request.state)}</span
            >{#if request.error && ['failed', 'uncertain'].includes(request.state)}<span
                class="mt-1 block text-xs text-danger">{request.error}</span
              >{/if}{#if user.role === 'admin'}<span
                class="mt-1 block text-[10px] text-muted"
                >{request.username}</span
              >{/if}</span
          >
        </div>
        <div class="flex flex-wrap items-center gap-2">
          {#if request.state === 'pending' && user.role === 'admin'}<Button
              size="sm"
              disabled={busy !== null}
              onclick={() => void act(request, 'approve')}>Approve</Button
            ><Button
              variant="secondary"
              size="sm"
              disabled={busy !== null}
              onclick={() => void act(request, 'deny')}>Decline</Button
            >{/if}
          {#if ['pending', 'failed', 'uncertain'].includes(request.state)}<Button
              variant="ghost"
              size="sm"
              disabled={busy !== null}
              aria-label={`Cancel the request for ${request.title}`}
              onclick={() => void act(request, 'cancel')}>Cancel</Button
            >{/if}
        </div>
      </article>{/each}
  </section>{/if}
