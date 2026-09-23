<script lang="ts">
  import SetupLink from './SetupLink.svelte';
  let { platform }: { platform: 'youtube' | 'twitch' | 'kick' } = $props();
</script>

<ol class="my-4 list-decimal space-y-2 pl-5 text-sm leading-6 text-muted">
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
      >. For a personal project, choose External and add each Google account
      under Audience → Test users while in Testing.
    </li>
    <li>
      Under <SetupLink href="https://console.cloud.google.com/auth/clients"
        >Clients</SetupLink
      >, create a <strong>Web application</strong>. Add the exact Authorized
      redirect URI shown below, then paste its client ID and secret here.
    </li>
    <li>
      Apply the application, then choose <strong>Connect YouTube</strong> on the
      YouTube page. Testing grants expire after seven days; use Google's <SetupLink
        href="https://developers.google.com/identity/protocols/oauth2/production-readiness/overview"
        >publishing guidance</SetupLink
      > for ongoing access.
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
      > with a unique name and a category. Choose <strong>Public</strong> as its
      client type. If a redirect URL is required, use
      <code>http://localhost</code>; device sign-in does not use a callback.
    </li>
    <li>
      Open Manage, copy the client ID and save it below. This flow needs only
      the client ID. Then choose <strong>Connect Twitch</strong> and approve the
      displayed code.
      <SetupLink
        href="https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/#device-code-grant-flow"
        >Twitch device sign-in help</SetupLink
      >.
    </li>
  {:else}
    <li>
      Enable two-factor authentication on your Kick account, then create an
      application in <SetupLink href="https://kick.com/settings/developer"
        >Kick Developer settings</SetupLink
      >.
    </li>
    <li>
      Paste its client ID and secret below. Thelxinoe uses application access
      for public channel metadata; viewers do not authorize their Kick accounts.
      If the form requires a redirect URL, enter your server's HTTPS origin;
      this flow does not call it.
      <SetupLink
        href="https://docs.kick.com/getting-started/generating-tokens-oauth2-flow"
        >Kick application access help</SetupLink
      >.
    </li>
  {/if}
</ol>
