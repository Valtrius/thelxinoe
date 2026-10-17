<script lang="ts">
  import { ChevronRight } from '@lucide/svelte';
  import SetupLink from './SetupLink.svelte';
  import FormField from '../ui/FormField.svelte';
  import CopyButton from '../ui/CopyButton.svelte';
  import { formControlClass } from '../ui/styles';
  import { reveal } from '../ui/reveal';

  // The guide starts folded once the application is configured; it slides
  // open and closed on the user's animation clock.
  let {
    platform,
    redirectUri,
    open = true,
  }: {
    platform: 'youtube' | 'twitch' | 'kick';
    redirectUri?: string | null;
    open?: boolean;
  } = $props();
  const panelId = $props.id();
  let expanded = $derived(open);
  const publicOrigin = $derived(
    redirectUri ? new URL(redirectUri).origin : null,
  );
  const steps = $derived({ youtube: 6, twitch: 3, kick: 2 }[platform]);
  const brandingPages = [
    {
      page: 'home',
      file: 'index',
      label: 'Application home page',
      action: 'Open home page',
    },
    {
      page: 'privacy',
      file: 'privacy',
      label: 'Application privacy policy link',
      action: 'Open privacy policy',
    },
  ] as const;
</script>

<div class="min-w-0">
  <button
    type="button"
    class="flex items-center gap-2 text-xs font-semibold text-accent"
    aria-expanded={expanded}
    aria-controls={panelId}
    onclick={() => (expanded = !expanded)}
  >
    <ChevronRight
      size={14}
      class={[
        'shrink-0 transition-transform duration-(--animation-duration)',
        expanded && 'rotate-90',
      ]}
      aria-hidden="true"
    />
    Setup guide
    <span class="font-normal text-muted">· {steps} steps</span>
  </button>
  {#if expanded}
    <!-- flow-root keeps the list margins inside the measured box; otherwise
         the slide ends short of the final height and the layout snaps. -->
    <div id={panelId} class="flow-root" transition:reveal>
      <ol class="my-3 list-decimal space-y-2 pl-5 text-xs leading-6 text-muted">
        {#if platform === 'youtube'}
          <li>
            Create or select a project in <SetupLink
              href="https://console.cloud.google.com/">Google Cloud</SetupLink
            >
            and enable <SetupLink
              href="https://console.cloud.google.com/apis/library/youtube.googleapis.com"
              >YouTube Data API v3</SetupLink
            >.
          </li>
          <li>
            Configure the app name, support email and audience in <SetupLink
              href="https://console.cloud.google.com/auth/overview"
              >Google Auth Platform</SetupLink
            >. For a personal project, choose External and add each Google
            account under Audience → Test users while in Testing.
          </li>
          <li>
            In <SetupLink href="https://console.cloud.google.com/auth/branding"
              >Branding</SetupLink
            >, use the public pages below for the application home page and
            privacy policy. Keep both reachable over HTTPS without signing in,
            including through your reverse proxy. Add your domain under
            Authorized domains and verify ownership if Google requests it.
            {#if publicOrigin}
              <div class="mt-3 grid gap-3 @min-[640px]/settings:grid-cols-2">
                {#each brandingPages as link (link.page)}
                  {@const href = `${publicOrigin}/about/${link.file}.html`}
                  <div class="min-w-0">
                    <div class="flex items-end gap-2">
                      <FormField class="mb-0 min-w-0 flex-1">
                        {link.label}
                        <input
                          class={formControlClass}
                          readonly
                          value={href}
                          onclick={(event) => event.currentTarget.select()}
                        />
                      </FormField>
                      <CopyButton
                        value={href}
                        label={`Copy ${link.label.toLowerCase()}`}
                      />
                    </div>
                    <SetupLink {href} publicPage={link.page}
                      >{link.action}</SetupLink
                    >
                  </div>
                {/each}
              </div>
            {:else}
              <p class="mt-2">
                Set the public server URL below to get these links.
              </p>
            {/if}
          </li>
          <li>
            Under <SetupLink
              href="https://console.cloud.google.com/auth/clients"
              >Clients</SetupLink
            >, create a <strong>Web application</strong>. Add the exact
            Authorized redirect URI shown below, then paste its client ID and
            secret here.
          </li>
          <li>
            In <SetupLink href="https://console.cloud.google.com/auth/audience"
              >Audience</SetupLink
            >, choose <strong>Publish app</strong> to switch to
            <strong>In production</strong>
            and remove the seven-day Testing expiry. Publishing and verification are
            separate; Google's <SetupLink
              href="https://developers.google.com/identity/protocols/oauth2/production-readiness/overview"
              >publishing guidance</SetupLink
            > explains verification requirements and unverified-app limits.
          </li>
          <li>
            Apply the application, then choose <strong>Connect YouTube</strong>
            on the YouTube page. If you connected while in Testing, disconnect and
            reconnect after publishing. Access can still expire or be revoked.
          </li>
        {:else if platform === 'twitch'}
          <li>
            Verify your Twitch email and enable two-factor authentication in <SetupLink
              href="https://www.twitch.tv/settings/security"
              >Security settings</SetupLink
            >.
          </li>
          <li>
            <SetupLink href="https://dev.twitch.tv/console/apps/create"
              >Register an application</SetupLink
            > with a unique name and a category. Choose <strong>Public</strong>
            as its client type. If a redirect URL is required, use
            <code>http://localhost</code>; device sign-in does not use a
            callback.
          </li>
          <li>
            Open Manage, copy the client ID and save it below. This flow needs
            only the client ID. Then choose <strong>Connect Twitch</strong> and
            approve the displayed code.
            <SetupLink
              href="https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/#device-code-grant-flow"
              >Twitch device sign-in help</SetupLink
            >.
          </li>
        {:else}
          <li>
            Enable two-factor authentication on your Kick account, then create
            an application in <SetupLink
              href="https://kick.com/settings/developer"
              >Kick Developer settings</SetupLink
            >.
          </li>
          <li>
            Paste its client ID and secret below. Thelxinoe uses application
            access for public channel metadata; viewers do not authorize their
            Kick accounts. If the form requires a redirect URL, enter your
            server's HTTPS origin; this flow does not call it.
            <SetupLink
              href="https://docs.kick.com/getting-started/generating-tokens-oauth2-flow"
              >Kick application access help</SetupLink
            >.
          </li>
        {/if}
      </ol>
    </div>
  {/if}
</div>
