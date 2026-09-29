<script lang="ts">
  import { ArrowUp, ArrowDownToLine, RefreshCw, Pause } from '@lucide/svelte';
  import { desktopUpdates, desktopUpdate } from './desktop-updates';
  import OrbitAction from './ui/OrbitAction.svelte';
  import VersionCard from './ui/VersionCard.svelte';
  let error = $state('');
  let starting = $state(false);
  const status = $derived($desktopUpdates);
  const busy = $derived(
    starting ||
      !status ||
      ['checking', 'downloading', 'waiting', 'installing'].includes(
        status.phase,
      ),
  );
  const failure = $derived(error || status?.error);
  const view = $derived.by(() => {
    if (status?.phase === 'downloading')
      return {
        title: 'Downloading desktop update',
        description: '',
        icon: ArrowDownToLine,
      };
    if (status?.phase === 'waiting')
      return {
        title: 'Waiting for playback to stop',
        description: 'Restarts automatically when ready.',
        icon: Pause,
      };
    if (status?.phase === 'installing')
      return { title: 'Restarting desktop', description: '', icon: RefreshCw };
    if (busy)
      return {
        title: 'Checking for updates',
        description: '',
        icon: RefreshCw,
      };
    if (failure)
      return {
        title: 'Desktop update could not complete',
        description: `${failure}\nClick to retry.`,
        icon: RefreshCw,
      };
    if (status?.release)
      return {
        title: `Update to ${status.release.version}`,
        description: 'Click to install and restart.',
        icon: ArrowUp,
      };
    return {
      title: status?.checked_at ? 'Up to date' : 'Check for updates',
      description: 'Click to check for updates.',
      icon: RefreshCw,
    };
  });
  async function apply() {
    if (busy) return;
    error = '';
    starting = true;
    try {
      if (status?.release)
        await desktopUpdate('apply', { version: status.release.version });
      else await desktopUpdate('check');
    } catch (e) {
      error = String(e);
    } finally {
      starting = false;
    }
  }
</script>

<VersionCard version={status?.installed ?? '—'} label="Desktop version">
  <OrbitAction
    label={busy
      ? view.title
      : status?.release
        ? `Update desktop to ${status.release.version}`
        : 'Check for desktop updates'}
    title={view.title}
    description={view.description}
    tone={failure && !busy
      ? 'warning'
      : busy || status?.release
        ? 'accent'
        : 'muted'}
    motion={busy ? 'working' : status?.release ? 'available' : 'none'}
    progress={status?.phase === 'downloading' && status.total
      ? (status.received / status.total) * 100
      : null}
    {busy}
    onclick={() => void apply()}
  >
    <view.icon size={17} strokeWidth={1.6} />
  </OrbitAction>
</VersionCard>
