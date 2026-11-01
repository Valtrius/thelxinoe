<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { Globe, Laptop, Smartphone } from '@lucide/svelte';
  import { api } from '../api';
  import { withVerification } from '../authentication';
  import { formatTimestamp } from '../display.svelte';
  import { LatestRequest } from '../latest-request';
  import { isServerUpdateInterruption } from '../server-updates';
  import { captureSession } from '../session';
  import Button from '../ui/Button.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import Notice from '../ui/Notice.svelte';
  import SettingRow from '../ui/SettingRow.svelte';
  import SettingsCard from '../ui/SettingsCard.svelte';
  import StatusChip from '../ui/StatusChip.svelte';
  import { cardBlockClass } from '../ui/styles';

  let { revision, sessionEnded } = $props<{
    revision: number;
    sessionEnded: () => void;
  }>();
  type Session = {
    id: string;
    name: string;
    transport: string;
    last_seen: number;
    remembered?: boolean;
  };
  let sessions = $state<Session[] | null>(null),
    current = $state(''),
    busy = $state(false),
    error = $state('');
  const requests = new LatestRequest();
  const ownsSession = captureSession();
  onDestroy(() => requests.invalidate());
  $effect(() => {
    void revision;
    untrack(() => void load());
  });
  const transportIcon = (transport: string) =>
    transport === 'desktop'
      ? Laptop
      : transport === 'mobile'
        ? Smartphone
        : Globe;
  async function load() {
    const latest = requests.begin();
    try {
      const value = await api<{ items: Session[]; current: string }>(
        '/auth/sessions',
      );
      if (!latest()) return;
      sessions = value.items;
      current = value.current;
      error = '';
    } catch (caught) {
      if (latest() && !isServerUpdateInterruption(caught))
        error = String(caught);
    }
  }
  async function revoke(session: Session) {
    busy = true;
    error = '';
    try {
      const revoked = await withVerification(async () => {
        await api(`/auth/sessions/${session.id}`, 'DELETE');
      });
      if (!revoked || !ownsSession()) return;
      if (session.id === current) sessionEnded();
      else await load();
    } catch (caught) {
      if (ownsSession()) error = String(caught);
    } finally {
      if (ownsSession()) busy = false;
    }
  }
</script>

<SettingsCard
  title="Your devices"
  description="Browsers and desktops signed in to your account. Revoking this device signs you out."
>
  {#if error}<div class={cardBlockClass}>
      <Notice variant="error" role="alert" class="mb-0">{error}</Notice>
    </div>{/if}
  {#if !sessions && !error}<div class={cardBlockClass}>
      <ContentSkeleton label="Loading devices" />
    </div>{/if}
  {#each sessions ?? [] as session (session.id)}
    {@const Icon = transportIcon(session.transport)}
    <SettingRow
      label={session.name}
      description={`${session.transport} · ${formatTimestamp(session.last_seen)}`}
    >
      {#snippet lead()}
        <Icon size={16} class="shrink-0 text-muted" aria-hidden="true" />
      {/snippet}
      {#snippet badge()}
        {#if session.id === current}<StatusChip tone="info" dot={false}
            >This device</StatusChip
          >{/if}
        {#if session.remembered}<StatusChip dot={false}>Remembered</StatusChip
          >{/if}
      {/snippet}
      <Button
        size="form"
        variant="ghost"
        disabled={busy}
        onclick={() => void revoke(session)}>Revoke</Button
      >
    </SettingRow>
  {/each}
</SettingsCard>
