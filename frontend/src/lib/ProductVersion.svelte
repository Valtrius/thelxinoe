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
  } from '@lucide/svelte';
  import OrbitAction from './ui/OrbitAction.svelte';
  import VersionCard from './ui/VersionCard.svelte';
  import ServerRollback from './ServerRollback.svelte';
  import {
    refreshServerUpdate,
    serverUpdateAction,
    serverUpdates,
  } from './server-updates';
  let { installed = '—' } = $props<{ installed?: string }>();
  const status = $derived($serverUpdates.status);
  const request = $derived(status?.request);
  const operation = $derived(
    request?.state === 'pending'
      ? status?.controller.items.find((item) => item.id === request.id)
      : status?.controller.items[0],
  );
  const stage = $derived(operation?.stage);
  const pending = $derived(request?.state === 'pending');
  const version = $derived(
    pending &&
      !['committed', 'restored', 'runtime-failure'].includes(stage ?? '')
      ? (request?.previous_version ?? operation?.previous_version ?? installed)
      : operation &&
          !['committed', 'restored', 'runtime-failure'].includes(
            operation.stage,
          ) &&
          status?.version === operation.version
        ? operation.previous_version
        : (status?.version ?? installed),
  );
  const candidate = $derived(status?.release?.version);
  type View = {
    title: string;
    description: string;
    icon: typeof RefreshCw;
    tone: 'muted' | 'accent' | 'success' | 'warning' | 'danger';
    motion: 'none' | 'available' | 'working' | 'restoring';
    action: 'check' | 'install' | 'connection' | null;
    label?: string;
  };
  const view = $derived.by((): View => {
    const busy = (
      title: string,
      description: string,
      icon = ShieldCheck,
      restoring = false,
    ): View => ({
      title,
      description,
      icon,
      tone: restoring ? 'warning' : 'accent',
      motion: restoring ? 'restoring' : 'working',
      action: null,
    });
    const attention = (title: string, description: string): View => ({
      title,
      description,
      icon: TriangleAlert,
      tone: 'danger',
      motion: 'none',
      action: 'connection',
      label: 'Check server connection',
    });
    if ($serverUpdates.offlineSince) {
      if ($serverUpdates.now - $serverUpdates.offlineSince > 120000)
        return attention(
          'The server has not returned',
          'Still retrying. Click to check now.',
        );
      return busy(
        'Reconnecting to the server',
        'Updates continue in the background.',
        Radio,
      );
    }
    if (status?.controller.error)
      return attention('Controller unavailable', 'Click to check again.');
    if ($serverUpdates.action)
      return busy(
        $serverUpdates.action === 'check'
          ? 'Checking for updates'
          : $serverUpdates.action === 'recover'
            ? 'Requesting recovery'
            : 'Queuing the update',
        '',
      );
    switch (stage) {
      case 'preparing':
        return busy(
          `Downloading ${operation?.download?.component === 'controller' ? 'controller' : 'server'} update`,
          '',
          ArrowDownToLine,
        );
      case 'snapshotting':
        return busy('Saving a recovery copy', '', Archive);
      case 'validating':
        return busy('Checking the update', '');
      case 'ready':
        if (pending)
          return busy(
            'Waiting for the server to be idle',
            'Waiting for playback and background work.',
            Pause,
          );
        break;
      case 'preparing-activation':
      case 'isolated-migration':
      case 'creating-successor':
      case 'handoff':
      case 'activating':
        return busy(
          `Installing ${operation?.version}`,
          'Restarting and reconnecting.',
          RefreshCw,
        );
      case 'recovering':
      case 'restoring-release':
        return busy('Restoring the previous version', '', Undo2, true);
      case 'runtime-failure':
      case 'recovery-required':
        return attention(
          'The update needs attention',
          operation?.error ?? 'Check the connection or use Rollback.',
        );
    }
    if (pending && !operation)
      return busy(
        'Waiting for the server to be idle',
        'Waiting for playback and background work.',
        Pause,
      );
    const error =
      $serverUpdates.error ||
      (request?.state === 'failed' &&
      (!operation || operation.id === request.id) &&
      (!candidate || candidate === request.version)
        ? request.error
        : '') ||
      status?.observation?.error;
    const relevantFailure =
      operation &&
      ['blocked', 'recovered', 'restored'].includes(operation.stage) &&
      (!candidate || candidate === operation.version);
    if (error || relevantFailure)
      return {
        title:
          relevantFailure && stage !== 'blocked'
            ? `${version} has been restored`
            : 'Update could not complete',
        description: `${error || operation?.error || 'The previous version is running.'}\n${candidate ? 'Click to retry the update.' : 'Click to check for updates.'}`,
        icon: relevantFailure && stage !== 'blocked' ? Undo2 : RefreshCw,
        tone: 'warning',
        motion: 'none',
        action: candidate ? 'install' : 'check',
        label: candidate ? 'Retry server update' : 'Check for server updates',
      };
    if (candidate)
      return {
        title: `Update to ${candidate}`,
        description: 'Click to install and restart.',
        icon: ArrowUp,
        tone: 'accent',
        motion: 'available',
        action: 'install',
        label: `Update server to ${candidate}`,
      };
    if (stage === 'committed' && operation?.version === status?.version)
      return {
        title: `Updated to ${version}`,
        description: 'Click to check for updates.',
        icon: RefreshCw,
        tone: 'success',
        motion: 'none',
        action: 'check',
      };
    return {
      title: !status
        ? 'Checking update status'
        : !status.configured
          ? 'Release checking unavailable'
          : status.observation?.checked_at
            ? 'Up to date'
            : 'Waiting for the first release check',
      description: !status?.configured
        ? 'Release signing key unavailable.'
        : 'Click to check for updates.',
      icon: RefreshCw,
      tone: 'muted',
      motion: 'none',
      action: status ? 'check' : null,
    };
  });
  function activate() {
    if (view.action === 'connection') void refreshServerUpdate();
    else if (view.action) void serverUpdateAction(view.action);
  }
</script>

<VersionCard {version} label="Product version">
  <OrbitAction
    label={view.label ??
      (view.action ? 'Check for server updates' : view.title)}
    title={view.title}
    description={view.description}
    tone={view.tone}
    motion={view.motion}
    progress={stage === 'preparing' && operation?.download?.total
      ? (operation.download.received / operation.download.total) * 100
      : null}
    busy={!view.action}
    onclick={activate}
  >
    <view.icon size={17} strokeWidth={1.6} />
  </OrbitAction>
  {#snippet tools()}<ServerRollback />{/snippet}
</VersionCard>
