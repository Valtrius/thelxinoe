<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { api } from './api';
  import {
    api as providerApi,
    type OnlineAccount,
    type KickFeed,
  } from './providers/api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';

  let { navigate, revision = 0 } = $props<{
    navigate: (section: string) => void;
    revision?: number;
  }>();
  let accounts = $state<Partial<Record<'youtube' | 'twitch', OnlineAccount>>>(
    {},
  );
  let kick = $state<KickFeed | null>(null);
  let busy = $state(false),
    error = $state(''),
    deleting = $state('');
  let request = 0;
  async function load() {
    const current = ++request;
    try {
      const [youtube, twitch, tracked] = await Promise.all([
        api<OnlineAccount>('/online/youtube'),
        api<OnlineAccount>('/online/twitch'),
        api<KickFeed>('/online/kick'),
      ]);
      if (current === request) {
        accounts = { youtube, twitch };
        kick = tracked;
      }
    } catch (caught) {
      if (current === request) throw caught;
    }
  }
  async function act(action: () => Promise<unknown>) {
    busy = true;
    error = '';
    try {
      await action();
      await load();
      deleting = '';
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  $effect(() => {
    void revision;
    untrack(() => void load().catch((caught) => (error = String(caught))));
  });
  onDestroy(() => request++);
</script>

<Panel>
  <h2>Your online accounts</h2>
  <p class="text-muted">
    Connections, watchlists and viewing history belong to your account on this
    server. A server administrator configures application credentials under
    Provider applications before accounts can connect.
  </p>
  {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
  {#each [['youtube', 'YouTube'], ['twitch', 'Twitch']] as [provider, label] (provider)}
    {@const account = accounts[provider as 'youtube' | 'twitch']}
    <div
      class="mb-4.5 flex flex-col items-start gap-4 border-b border-line py-4"
    >
      <div class="flex items-center gap-3">
        {#if account?.account.status === 'connected' && account.account.avatar_url}
          <img
            src={account.account.avatar_url}
            alt=""
            referrerpolicy="no-referrer"
            class="size-9 rounded-full border border-line object-cover"
          />
        {:else}
          <span
            class="grid size-9 place-items-center rounded-full border border-line bg-surface-soft text-xs font-semibold text-muted"
            >{label[0]}</span
          >
        {/if}
        <div>
          <strong>{label}</strong>
          <p class="mt-1.5 mb-0 text-muted">
            {account?.account.status === 'connected'
              ? account.account.display_name
              : account?.account.status === 'reconnect_required'
                ? 'Reconnect to resume synchronization.'
                : 'Not connected'}
          </p>
        </div>
      </div>
      <div class="flex flex-wrap gap-2">
        <Button
          variant="secondary"
          size="form"
          disabled={busy ||
            !account?.configured ||
            (provider === 'youtube' && !account.linking_available)}
          onclick={() =>
            void act(async () => {
              if (provider === 'youtube') {
                await providerApi.connectYoutube();
                navigate(label);
              } else {
                await providerApi.startTwitchAuth();
                navigate(label);
              }
            })}
          >{account?.account.status === 'connected' ? 'Reconnect' : 'Connect'}
          {label}</Button
        >
        {#if account && account.account.status !== 'disconnected'}<Button
            variant="secondary"
            size="form"
            disabled={busy}
            onclick={() => void act(() => api(`/online/${provider}`, 'DELETE'))}
            >Disconnect {label}</Button
          >{/if}
        <Button
          variant="secondary"
          size="form"
          disabled={busy}
          onclick={() => (deleting = provider)}>Delete {label} data…</Button
        >
      </div>
    </div>
    {#if deleting === provider}<Notice variant="error">
        <p>
          Disconnect {label} and delete your {provider === 'youtube'
            ? 'feed, watchlists, pins and history'
            : 'cached channels and history'} from this server?
        </p>
        <Button
          variant="secondary"
          size="form"
          disabled={busy}
          onclick={() =>
            void act(() => api(`/online/${provider}/data`, 'DELETE'))}
          >Delete my {label} data</Button
        ><Button variant="secondary" size="form" onclick={() => (deleting = '')}
          >Cancel</Button
        >
      </Notice>{/if}
  {/each}
  <div class="mb-4.5 flex flex-col items-start gap-4 border-b border-line py-4">
    <div>
      <strong>Kick</strong>
      <p class="mt-1.5 mb-0 text-muted">
        {kick?.items.length ?? 0} tracked channels. {kick?.connected
          ? 'Synchronization enabled.'
          : 'Synchronization paused.'}
      </p>
    </div>
    <div class="flex flex-wrap gap-2">
      <Button
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() =>
          void act(() =>
            api(
              kick?.connected ? '/online/kick' : '/online/kick/connect',
              kick?.connected ? 'DELETE' : 'POST',
            ),
          )}>{kick?.connected ? 'Pause' : 'Resume'} Kick synchronization</Button
      ><Button variant="secondary" size="form" onclick={() => navigate('Kick')}
        >Manage channels</Button
      ><Button
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() => (deleting = 'kick')}>Delete Kick data…</Button
      >
    </div>
  </div>
  {#if deleting === 'kick'}<Notice variant="error">
      <p>Delete your tracked Kick channels and history?</p>
      <Button
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() => void act(() => api('/online/kick/data', 'DELETE'))}
        >Delete my Kick data</Button
      ><Button variant="secondary" size="form" onclick={() => (deleting = '')}
        >Cancel</Button
      >
    </Notice>{/if}
</Panel>
