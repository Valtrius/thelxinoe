<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import {
    api as providerApi,
    type OnlineAccount,
    type KickFeed,
  } from './providers/api';
  let { navigate } = $props<{ navigate: (section: string) => void }>();
  let accounts = $state<Partial<Record<'youtube' | 'twitch', OnlineAccount>>>(
    {},
  );
  let kick = $state<KickFeed | null>(null);
  let busy = $state(false),
    error = $state(''),
    deleting = $state('');
  async function load() {
    const [youtube, twitch, tracked] = await Promise.all([
      api<OnlineAccount>('/online/youtube'),
      api<OnlineAccount>('/online/twitch'),
      api<KickFeed>('/online/kick'),
    ]);
    accounts = { youtube, twitch };
    kick = tracked;
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
  onMount(() => {
    void act(load);
  });
</script>

<section class="panel">
  <h2>Your online accounts</h2>
  <p class="muted">
    Connections, watchlists and viewing history belong to your account on this
    server. A server administrator configures application credentials under
    Provider applications before accounts can connect.
  </p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#each [['youtube', 'YouTube'], ['twitch', 'Twitch']] as [provider, label] (provider)}
    {@const account = accounts[provider as 'youtube' | 'twitch']}
    <div class="setting-row">
      <div>
        <strong>{label}</strong>
        <p class="muted">
          {account?.account.status === 'connected'
            ? account.account.display_name
            : account?.account.status === 'reconnect_required'
              ? 'Reconnect to resume synchronization.'
              : 'Not connected'}
        </p>
      </div>
      <div class="actions">
        <button
          class="secondary"
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
          {label}</button
        >
        {#if account?.account.status !== 'disconnected'}<button
            class="secondary"
            disabled={busy}
            onclick={() => void act(() => api(`/online/${provider}`, 'DELETE'))}
            >Disconnect {label}</button
          >{/if}
        <button
          class="secondary"
          disabled={busy}
          onclick={() => (deleting = provider)}>Delete {label} data…</button
        >
      </div>
    </div>
    {#if deleting === provider}<div class="error">
        <p>
          Disconnect {label} and delete your {provider === 'youtube'
            ? 'feed, watchlists, pins and history'
            : 'cached channels and history'} from this server?
        </p>
        <button
          class="secondary"
          disabled={busy}
          onclick={() =>
            void act(() => api(`/online/${provider}/data`, 'DELETE'))}
          >Delete my {label} data</button
        ><button class="secondary" onclick={() => (deleting = '')}
          >Cancel</button
        >
      </div>{/if}
  {/each}
  <div class="setting-row">
    <div>
      <strong>Kick</strong>
      <p class="muted">
        {kick?.items.length ?? 0} tracked channels. {kick?.connected
          ? 'Synchronization enabled.'
          : 'Synchronization paused.'}
      </p>
    </div>
    <div class="actions">
      <button
        class="secondary"
        disabled={busy}
        onclick={() =>
          void act(() =>
            api(
              kick?.connected ? '/online/kick' : '/online/kick/connect',
              kick?.connected ? 'DELETE' : 'POST',
            ),
          )}>{kick?.connected ? 'Pause' : 'Resume'} Kick synchronization</button
      ><button class="secondary" onclick={() => navigate('Kick')}
        >Manage channels</button
      ><button
        class="secondary"
        disabled={busy}
        onclick={() => (deleting = 'kick')}>Delete Kick data…</button
      >
    </div>
  </div>
  {#if deleting === 'kick'}<div class="error">
      <p>Delete your tracked Kick channels and history?</p>
      <button
        class="secondary"
        disabled={busy}
        onclick={() => void act(() => api('/online/kick/data', 'DELETE'))}
        >Delete my Kick data</button
      ><button class="secondary" onclick={() => (deleting = '')}>Cancel</button>
    </div>{/if}
</section>

<style>
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .setting-row {
    align-items: flex-start;
    gap: 16px;
    flex-direction: column;
  }
  .setting-row p {
    margin-top: 6px;
  }
</style>
