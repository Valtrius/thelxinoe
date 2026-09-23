<script lang="ts">
  import YoutubeSettings from '../YoutubeSettings.svelte';
  import ProviderApplicationSettings from './ProviderApplicationSettings.svelte';
  import Button from '../ui/Button.svelte';
  import SetupLink from './SetupLink.svelte';

  let {
    platform,
    admin,
    configured,
    onConfigured,
    onConnect,
  }: {
    platform: 'youtube' | 'twitch' | 'kick';
    admin: boolean;
    configured: boolean;
    onConfigured: () => void;
    onConnect?: () => void;
  } = $props();
</script>

<section
  class="provider-setup border border-line bg-surface p-4 shadow-panel sm:p-6"
  aria-label={`${platform} connection guide`}
>
  <div
    class="mx-auto max-w-3xl [&_.panel]:mb-0 [&_.panel]:border-0 [&_.panel]:bg-transparent [&_.panel]:p-0"
  >
    <h2 class="text-xl">
      {platform === 'youtube'
        ? 'Connect YouTube'
        : platform === 'twitch'
          ? 'Connect Twitch'
          : 'Track Kick channels'}
    </h2>
    {#if platform === 'kick'}
      <ol class="my-4 list-decimal space-y-2 pl-5 text-sm leading-6 text-muted">
        <li>
          Find a public channel on <SetupLink href="https://kick.com/"
            >Kick</SetupLink
          > and copy its name or channel URL.
        </li>
        <li>
          Open <strong>Manage channels</strong>, paste it and choose
          <strong>Add</strong>. Your tracked list is private; no Kick sign-in is
          needed.
        </li>
        <li>
          Choose Play when the channel is live. Following lists are not imported
          from Kick.
        </li>
      </ol>
    {:else if configured}
      <p class="my-4 text-sm leading-6 text-muted">
        {#if platform === 'youtube'}
          Choose Connect YouTube, sign in to Google and approve read-only
          YouTube access. If the server opens in your browser, sign in to
          Thelxinoe there first and choose Connect YouTube again. Return here
          after approval; your subscriptions will sync automatically.
        {:else}
          Choose Connect Twitch, open the verification page and enter the code
          shown here. Sign in to Twitch and approve followed-channel access,
          then return here to see followed channels that are live.
        {/if}
      </p>
    {:else if !admin}
      <p class="my-4 text-sm leading-6 text-muted">
        Ask your Thelxinoe administrator to configure {platform === 'youtube'
          ? 'the Google application and public server URL'
          : 'a Public Twitch application'} in Settings → Provider applications. Once
        ready, return here to connect your own account.
      </p>
    {/if}
    {#if onConnect && (configured || platform === 'kick')}
      <Button onclick={onConnect}
        >{platform === 'youtube'
          ? 'Connect YouTube'
          : platform === 'twitch'
            ? 'Connect Twitch'
            : 'Manage channels'}</Button
      >
    {/if}
    {#if !configured}
      {#if platform === 'kick'}
        <details class="mt-4 text-sm">
          <summary class="cursor-pointer text-muted"
            >Optional: show live status, titles and thumbnails</summary
          >
          {#if admin}
            <div class="mt-3">
              <ProviderApplicationSettings platform="kick" {onConfigured} />
            </div>
          {:else}
            <p class="mt-3 text-muted">
              Ask your Thelxinoe administrator to add a Kick application in
              Settings → Provider applications. You can already track and play
              public channels.
            </p>
          {/if}
        </details>
      {:else if admin}
        <div class="mt-4 [&_.panel>h2]:hidden">
          {#if platform === 'youtube'}
            <YoutubeSettings {onConfigured} />
          {:else}
            <ProviderApplicationSettings platform="twitch" {onConfigured} />
          {/if}
        </div>
      {/if}
    {/if}
  </div>
</section>
