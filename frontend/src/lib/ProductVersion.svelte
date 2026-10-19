<script lang="ts">
  import {
    ArrowUp,
    ArrowDownToLine,
    Archive,
    ShieldCheck,
    RefreshCw,
    Radio,
    Undo2,
    Pause,
    TriangleAlert,
    X,
  } from '@lucide/svelte';
  import Button from './ui/Button.svelte';
  import OrbitAction from './ui/OrbitAction.svelte';
  import VersionCard from './ui/VersionCard.svelte';
  import ServerRollback from './ServerRollback.svelte';
  import {
    refreshServerUpdate,
    serverUpdateAction,
    serverUpdateCard,
    serverUpdates,
  } from './server-updates';
  let { installed = '—', inline = false } = $props<{
    installed?: string;
    inline?: boolean;
  }>();
  const card = $derived(serverUpdateCard($serverUpdates, installed));
  const view = $derived(card.view);
  const icons = {
    available: ArrowUp,
    download: ArrowDownToLine,
    snapshot: Archive,
    check: ShieldCheck,
    refresh: RefreshCw,
    reconnect: Radio,
    restore: Undo2,
    wait: Pause,
    alert: TriangleAlert,
  };
  const Icon = $derived(icons[view.icon]);
  function activate() {
    if (view.action === 'connection') void refreshServerUpdate();
    else if (view.action) void serverUpdateAction(view.action);
  }
</script>

<VersionCard version={card.version} label="Product version" {inline}>
  <OrbitAction
    label={view.label ??
      (view.action ? 'Check for server updates' : view.title)}
    title={view.title}
    description={view.description}
    tone={view.tone}
    motion={view.motion}
    progress={card.progress}
    busy={!view.action}
    onclick={activate}
  >
    <Icon size={17} strokeWidth={1.6} />
  </OrbitAction>
  {#snippet tools()}
    {#if card.cancellable}<Button
        variant="ghost"
        size="sm"
        aria-label="Cancel queued server update"
        onclick={() => void serverUpdateAction('cancel')}
        ><X size={14} />Cancel</Button
      >{/if}
    {#if card.dismissible}<Button
        variant="ghost"
        size="compact-icon"
        aria-label="Dismiss failed server update"
        title="Dismiss"
        onclick={() =>
          void serverUpdateAction('dismiss', card.dismissible ?? undefined)}
        ><X size={14} /></Button
      >{/if}
    <ServerRollback />
  {/snippet}
</VersionCard>
