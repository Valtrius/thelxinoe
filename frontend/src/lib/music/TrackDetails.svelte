<script lang="ts">
  import { Play } from '@lucide/svelte';
  import Modal from '../ui/Modal.svelte';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import MediaActions from '../MediaActions.svelte';
  import MediaOperations from '../MediaOperations.svelte';
  import { time } from '../playback';
  import { formatBytes } from '../providers/utils';
  import { mediaFacts } from '../title/media-facts';
  import { loadItem, type CatalogItem } from '../title/catalog';
  import { eyebrowTextClass } from '../ui/styles';
  import { trackNumber, type MusicItem } from './library';

  let {
    track,
    admin,
    play,
    close,
    changed,
  }: {
    track: MusicItem;
    admin: boolean;
    play: (fileId: string) => void;
    close: () => void;
    changed: () => void;
  } = $props();
  let detail = $state<CatalogItem | null>(null),
    error = $state('');
  $effect(() => {
    const id = track.id;
    detail = null;
    error = '';
    loadItem(id).then(
      (item) => {
        if (track.id === id) detail = item;
      },
      (e) => {
        if (track.id === id) error = String(e);
      },
    );
  });
  const context = $derived(
    [
      track.artist,
      track.album,
      trackNumber(track) ? `Track ${trackNumber(track)}` : '',
    ]
      .filter(Boolean)
      .join(' · '),
  );
</script>

<Modal title={track.title} onClose={close}>
  {#if context}<p class="-mt-2 mb-4 text-xs text-muted">{context}</p>{/if}
  {#if error}<Notice variant="error" role="alert">{error}</Notice
    >{:else if !detail}<ContentSkeleton label="Loading track details" />{:else}
    <h3 class={`${eyebrowTextClass} mb-1`}>Files</h3>
    {#each detail.files ?? [] as file (file.id)}
      {@const facts = mediaFacts(file.probe)}
      {@const length = Number(file.probe.format?.duration)}
      {@const size = Number(file.probe.format?.size)}
      <div
        class="flex items-center justify-between gap-3 border-t border-line py-2.5 text-xs"
      >
        <div class="min-w-0">
          {file.edition || 'Original edition'}
          <small
            >{[
              facts.audio,
              length > 0 ? time(length) : '',
              size > 0 ? formatBytes(size) : '',
              file.present ? '' : 'Missing',
            ]
              .filter(Boolean)
              .join(' · ')}</small
          >
        </div>
        {#if file.present}<Button
            size="sm"
            onclick={() => {
              play(file.id);
              close();
            }}><Play size={13} fill="currentColor" />Play</Button
          >{/if}
      </div>
    {/each}
    <MediaActions id={track.id} kind="track" />
    {#if admin}<MediaOperations id={track.id} kind="track" {changed} />{/if}
  {/if}
</Modal>
