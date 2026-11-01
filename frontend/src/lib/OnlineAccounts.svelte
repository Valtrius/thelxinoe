<script lang="ts">
  import { revisions } from './server-events.svelte';
  import Notice from './ui/Notice.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { api } from './api';
  import {
    api as providerApi,
    type OnlineAccount,
    type KickFeed,
  } from './providers/api';
  import Button from './ui/Button.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import AttentionDot from './ui/AttentionDot.svelte';
  import { activity } from './activity';
  import { providers } from './providers/availability';
  import { cardBlockClass } from './ui/styles';
  import { reveal } from './ui/reveal';

  let { navigate, configureProviders } = $props<{
    navigate: (section: string) => void;
    configureProviders?: () => void;
  }>();
  const linked = [
    ['youtube', 'YouTube'],
    ['twitch', 'Twitch'],
  ] as const;
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
        $providers.youtube ? api<OnlineAccount>('/online/youtube') : undefined,
        $providers.twitch ? api<OnlineAccount>('/online/twitch') : undefined,
        $providers.kick ? api<KickFeed>('/online/kick') : null,
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
    void revisions.account;
    void $providers;
    untrack(() => void load().catch((caught) => (error = String(caught))));
  });
  onDestroy(() => request++);
  function statusTone(account?: OnlineAccount) {
    return account?.account.status === 'connected'
      ? 'ok'
      : account?.account.status === 'reconnect_required'
        ? 'warn'
        : 'neutral';
  }
  function statusLabel(account?: OnlineAccount) {
    return account?.account.status === 'connected'
      ? 'Connected'
      : account?.account.status === 'reconnect_required'
        ? 'Reconnect required'
        : 'Not connected';
  }
</script>

<div class="col-span-full grid min-w-0 gap-5 empty:hidden">
  {#if Object.values(accounts).some((account) => account && !account.configured)}
    <Notice class="mb-0">
      {#if configureProviders}<a
          href="#provider-applications"
          class="text-accent underline underline-offset-2"
          onclick={(event) => {
            event.preventDefault();
            configureProviders();
          }}
          >Connections unavailable until an admin sets up the provider
          applications.</a
        >{:else}Connections unavailable until an admin sets up the provider
        applications.{/if}
    </Notice>
  {/if}
  {#if error}<Notice variant="error" role="alert" class="mb-0">{error}</Notice
    >{/if}
</div>
<SettingsCard
  title="Connected accounts"
  description="Each account syncs its own feed into Thelxinoe."
>
  {#each linked as [provider, label] (provider)}
    {#if $providers[provider]}
      {@const account = accounts[provider]}
      <SettingRow
        {label}
        description={account?.account.status === 'connected'
          ? account.account.display_name
          : account?.account.status === 'reconnect_required'
            ? 'Reconnect to resume synchronization.'
            : 'Connect to sync your subscriptions and history.'}
      >
        {#snippet lead()}
          {#if account?.account.status === 'connected' && account.account.avatar_url}
            <img
              src={account.account.avatar_url}
              alt=""
              referrerpolicy="no-referrer"
              class="size-9 shrink-0 rounded-full border border-line object-cover"
            />
          {:else}
            <span
              class="grid size-9 shrink-0 place-items-center rounded-full border border-line bg-surface-soft text-xs font-semibold text-muted"
              >{label[0]}</span
            >
          {/if}
        {/snippet}
        {#snippet badge()}
          <StatusChip tone={statusTone(account)}
            >{statusLabel(account)}</StatusChip
          >
          <AttentionDot
            activity={$activity.filter(
              (item) => item.target === 'online' && item.resource === provider,
            )}
          />
        {/snippet}
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
            variant="ghost"
            size="form"
            disabled={busy}
            onclick={() => void act(() => api(`/online/${provider}`, 'DELETE'))}
            >Disconnect {label}</Button
          >{/if}
      </SettingRow>
    {/if}
  {/each}
  {#if $providers.kick}
    <SettingRow
      label="Kick"
      description={`${kick?.items.length ?? 0} tracked ${(kick?.items.length ?? 0) === 1 ? 'channel' : 'channels'} · no sign-in required`}
    >
      {#snippet lead()}
        <span
          class="grid size-9 shrink-0 place-items-center rounded-full border border-line bg-surface-soft text-xs font-semibold text-muted"
          >K</span
        >
      {/snippet}
      {#snippet badge()}
        <StatusChip tone={kick?.connected ? 'ok' : 'neutral'}
          >{kick?.connected ? 'Synchronizing' : 'Paused'}</StatusChip
        >
      {/snippet}
      <Button variant="secondary" size="form" onclick={() => navigate('Kick')}
        >Manage channels</Button
      >
      <Button
        variant="ghost"
        size="form"
        disabled={busy}
        onclick={() =>
          void act(() =>
            api(
              kick?.connected ? '/online/kick' : '/online/kick/connect',
              kick?.connected ? 'DELETE' : 'POST',
            ),
          )}>{kick?.connected ? 'Pause' : 'Resume'} Kick synchronization</Button
      >
    </SettingRow>
  {/if}
</SettingsCard>
<SettingsCard
  title="Imported data"
  description="Deleting removes what a provider imported into this server. Your library and other accounts stay."
>
  {#each linked as [provider, label] (provider)}
    {#if $providers[provider]}
      <SettingRow
        label={`${label} data`}
        description={provider === 'youtube'
          ? 'Your feed, watchlists, pins and history from YouTube.'
          : 'Cached Twitch channels and history.'}
      >
        <Button
          variant="danger"
          size="form"
          disabled={busy}
          onclick={() => (deleting = provider)}>Delete {label} data…</Button
        >
      </SettingRow>
      {#if deleting === provider}
        <div class={cardBlockClass} transition:reveal>
          <Notice variant="error" class="mb-0">
            <p>
              Disconnect {label} and delete your {provider === 'youtube'
                ? 'feed, watchlists, pins and history'
                : 'cached channels and history'} from this server?
            </p>
            <div class="flex flex-wrap gap-2">
              <Button
                variant="danger"
                size="form"
                disabled={busy}
                onclick={() =>
                  void act(() => api(`/online/${provider}/data`, 'DELETE'))}
                >Delete my {label} data</Button
              ><Button
                variant="secondary"
                size="form"
                onclick={() => (deleting = '')}>Cancel</Button
              >
            </div>
          </Notice>
        </div>
      {/if}
    {/if}
  {/each}
  {#if $providers.kick}
    <SettingRow
      label="Kick data"
      description="Your tracked Kick channels and history."
    >
      <Button
        variant="danger"
        size="form"
        disabled={busy}
        onclick={() => (deleting = 'kick')}>Delete Kick data…</Button
      >
    </SettingRow>
    {#if deleting === 'kick'}
      <div class={cardBlockClass} transition:reveal>
        <Notice variant="error" class="mb-0">
          <p>Delete your tracked Kick channels and history?</p>
          <div class="flex flex-wrap gap-2">
            <Button
              variant="danger"
              size="form"
              disabled={busy}
              onclick={() => void act(() => api('/online/kick/data', 'DELETE'))}
              >Delete my Kick data</Button
            ><Button
              variant="secondary"
              size="form"
              onclick={() => (deleting = '')}>Cancel</Button
            >
          </div>
        </Notice>
      </div>
    {/if}
  {/if}
</SettingsCard>
