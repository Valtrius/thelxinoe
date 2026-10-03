<script lang="ts">
  import { untrack } from 'svelte';
  import {
    ArrowUp,
    ArrowDownToLine,
    Check,
    Pause,
    RefreshCw,
    ShieldCheck,
    TriangleAlert,
    Undo2,
  } from '@lucide/svelte';
  import { api } from './api';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import OrbitAction from './ui/OrbitAction.svelte';
  import Switch from './ui/Switch.svelte';
  import { formControlClass } from './ui/styles';
  import { toolBusy, toolName, type ServerTool } from './server-tools';

  let {
    item,
    supported,
    refresh,
  }: {
    item: ServerTool;
    supported: boolean;
    refresh: () => Promise<void>;
  } = $props();
  let policy = $state(untrack(() => item.policy));
  let channel = $state(untrack(() => item.channel));
  let pinned = $state(untrack(() => item.pinned));
  let saving = $state(false);
  let preferences = $state<{ submit: () => Promise<void> }>();
  let action = $state('');
  let error = $state('');
  let revision = untrack(() => item.revision);
  $effect(() => {
    if (!saving && revision !== item.revision) {
      policy = item.policy;
      channel = item.channel;
      pinned = item.pinned;
      revision = item.revision;
    }
  });
  const name = $derived(toolName(item.id));
  const busy = $derived(toolBusy(item));
  const available = $derived(
    item.candidate && item.candidate.id !== item.installed?.candidate_id,
  );
  const stale = $derived(
    !item.checked_at ||
      Date.now() / 1000 - item.checked_at > 86400 ||
      !!item.check_error,
  );
  const progress = $derived(
    item.job?.reason ??
      {
        checking: 'Checking for updates',
        installing: 'Installing',
        downloading: 'Downloading',
        verifying: 'Verifying',
        validating: 'Validating',
        queued: 'Queued',
        waiting: 'Waiting',
        activating: 'Activating',
      }[item.job?.stage ?? ''],
  );
  type View = {
    title: string;
    description: string;
    icon: typeof RefreshCw;
    tone: 'muted' | 'accent' | 'success' | 'warning' | 'danger';
    motion: 'none' | 'available' | 'working';
    action: 'check' | 'install' | 'repair' | null;
    label: string;
  };
  const view = $derived.by((): View => {
    const check = {
      title: !item.installed
        ? 'Not installed'
        : stale
          ? 'Update status unknown'
          : 'Up to date',
      description:
        (stale && item.checked_at
          ? 'Last checked ' +
            new Date(item.checked_at * 1000).toLocaleString() +
            '\n'
          : '') + 'Click to check for updates.',
      icon: !stale && item.installed ? Check : RefreshCw,
      tone: !stale && item.installed ? 'success' : 'muted',
      motion: 'none',
      action: 'check',
      label: 'Check for ' + name + ' updates',
    } satisfies View;
    if (!supported)
      return {
        ...check,
        title: 'Managed tools require the Linux x86-64 server image',
        description: '',
        icon: TriangleAlert,
        tone: 'warning',
        action: null,
      };
    if (
      action ||
      (busy && !(item.job?.stage === 'waiting' && !item.job.manual))
    )
      return {
        title:
          action === 'check'
            ? 'Checking for updates'
            : (progress ?? 'Queuing the update'),
        description: item.job?.total
          ? (item.job.received / 1024 ** 2).toFixed(1) +
            ' / ' +
            (item.job.total / 1024 ** 2).toFixed(1) +
            ' MB'
          : '',
        icon:
          item.job?.stage === 'downloading'
            ? ArrowDownToLine
            : item.job?.stage === 'waiting'
              ? Pause
              : ShieldCheck,
        tone: 'accent',
        motion: 'working',
        action: null,
        label: progress ?? 'Checking for ' + name + ' updates',
      };
    if (item.integrity_error)
      return {
        ...check,
        title: 'Tool needs repair',
        description:
          item.integrity_error +
          (pinned ? '\nUnpin to repair.' : '\nClick to repair.'),
        icon: TriangleAlert,
        tone: 'danger',
        action: item.installed && !pinned ? 'repair' : 'check',
        label: item.installed && !pinned ? 'Repair ' + name : check.label,
      };
    const failure =
      error ||
      (item.job?.stage === 'failed' && item.job.error) ||
      item.check_error;
    if (failure)
      return {
        ...check,
        title: 'Update could not complete',
        description: [
          failure,
          available && item.candidate
            ? 'Available: ' + item.candidate.version
            : '',
          check.description,
        ]
          .filter(Boolean)
          .join('\n'),
        icon: TriangleAlert,
        tone: 'warning',
      };
    if (available && item.candidate)
      return {
        title:
          (pinned
            ? 'Available: '
            : item.installed
              ? 'Update to '
              : 'Install ') + item.candidate.version,
        description: [
          item.candidate.version === item.installed?.version
            ? item.id === 'streamlink'
              ? 'Dependency update.'
              : 'Build update.'
            : '',
          busy ? progress : '',
          item.held === item.candidate.id ? 'Held after rollback.' : '',
          pinned ? 'Pinned. Click to check for updates.' : 'Click to install.',
        ]
          .filter(Boolean)
          .join('\n'),
        icon: ArrowUp,
        tone: 'accent',
        motion: 'available',
        action: pinned ? 'check' : 'install',
        label: pinned
          ? check.label
          : (item.installed ? 'Update ' : 'Install ') +
            name +
            ' to ' +
            item.candidate.version,
      };
    if (busy)
      return {
        ...check,
        title: progress ?? 'Waiting',
        icon: Pause,
        tone: 'muted',
      };
    return check;
  });

  async function perform(kind: string, candidateId?: string) {
    if (action) return;
    action = kind;
    error = '';
    try {
      await api('/admin/tools/' + item.id + '/' + kind, 'POST', {
        candidate_id: candidateId,
      });
    } catch (caught) {
      error = String(caught);
    } finally {
      await refresh();
      action = '';
    }
  }
  function activate() {
    if (view.action)
      void perform(
        view.action,
        view.action === 'repair'
          ? item.installed?.candidate_id
          : view.action === 'install'
            ? item.candidate?.id
            : undefined,
      );
  }
</script>

<tr class="border-t border-line align-top">
  <th scope="row" class="py-3 pr-3 text-left text-sm font-medium">{name}</th>
  <td class="py-3 pr-3 text-sm tabular-nums"
    >{item.installed?.version ?? 'Not installed'}</td
  >
  <td class="py-3 pr-3">
    <Switch
      size="sm"
      checked={pinned}
      onCheckedChange={(next) => {
        pinned = next;
        void preferences?.submit();
      }}
    >
      <span class="sr-only">Pin {name}</span>
    </Switch>
  </td>
  <td class="py-2 pr-3">
    <AutoSaveForm
      bind:this={preferences}
      label={name + ' preferences'}
      bind:busy={saving}
      value={{ policy, channel, pinned }}
      onRevert={(value) => {
        policy = value.policy;
        channel = value.channel;
        pinned = value.pinned;
      }}
      onsave={async (value) => {
        await api('/admin/tools/' + item.id + '/settings', 'POST', value);
        await refresh();
      }}
      class="flex flex-wrap items-center gap-2"
    >
      {#snippet children(save)}
        <ExclusiveChoiceGroup
          ariaLabel={item.id + ' update policy'}
          value={policy}
          choices={[
            { value: 'inherit', label: 'Inherit' },
            { value: 'notify', label: 'Notify' },
            { value: 'automatic', label: 'Automatic' },
          ]}
          onChange={(next) => {
            policy = next;
            void save();
          }}
        />
        {#if item.id === 'yt-dlp'}
          <select
            aria-label="yt-dlp channel"
            class={[formControlClass, 'h-8! w-auto! py-1! text-xs!']}
            bind:value={channel}
          >
            <option value="nightly">Nightly</option>
            <option value="stable">Stable</option>
          </select>
        {:else}
          <span class="text-xs text-muted"
            >{item.channel === 'lts' ? 'LTS' : 'Stable'}</span
          >
        {/if}
      {/snippet}
    </AutoSaveForm>
  </td>
  <td class="w-16 py-2 text-xs">
    <div class="flex items-center gap-1">
      <OrbitAction
        label={view.label}
        title={view.title}
        description={view.description}
        tone={view.tone}
        motion={view.motion}
        busy={!view.action}
        progress={busy && item.job?.stage === 'downloading' && item.job.total
          ? (item.job.received / item.job.total) * 100
          : null}
        onclick={activate}
      >
        <view.icon size={17} strokeWidth={1.6} />
        {#snippet details()}
          {#if available && item.candidate?.notes_url}
            <a
              class="mt-2 inline-block text-xs text-accent underline underline-offset-2 normal-case tracking-normal"
              href={item.candidate.notes_url}
              target="_blank"
              rel="noreferrer">Release notes</a
            >
          {/if}
        {/snippet}
      </OrbitAction>
      {#if item.previous}
        <OrbitAction
          label={'Rollback ' + name}
          title={'Rollback to ' + item.previous.version}
          description={pinned
            ? 'Unpin to roll back.'
            : 'Restore the previous version.'}
          busy={!supported || busy || pinned || !!action}
          onclick={() => void perform('rollback', item.previous?.candidate_id)}
        >
          <Undo2 size={17} strokeWidth={1.6} />
        </OrbitAction>
      {/if}
    </div>
  </td>
</tr>
