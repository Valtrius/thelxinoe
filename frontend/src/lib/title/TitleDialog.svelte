<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from '../ui/Modal.svelte';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import MetadataEditor from '../MetadataEditor.svelte';
  import SegmentEditor from '../SegmentEditor.svelte';
  import EpisodeMapping from '../EpisodeMapping.svelte';
  import { startActivity } from '../activity';
  import { refreshAttention } from '../attention';
  import {
    executeOperation,
    prepareOperation,
    type CatalogFile,
  } from './catalog';

  export type TitleDialogKind =
    'match' | 'timestamps' | 'mapping' | 'delete' | 'replace';
  let {
    dialog,
    id,
    kind,
    title,
    files = [],
    kept = '',
    replace,
    close,
    changed,
  } = $props<{
    dialog: TitleDialogKind;
    id: string;
    kind: string;
    title: string;
    files?: CatalogFile[];
    /** Why Keep blocks deleting these files, if it does. */
    kept?: string;
    /** Searches for another release once a `replace` dialog deleted the files. */
    replace?: () => Promise<unknown>;
    close: () => void;
    changed?: () => void;
  }>();
  const present = $derived(files.filter((file: CatalogFile) => file.present));
  // Checking files can be cancelled; deleting them cannot.
  let preparing = $state(false),
    busy = $state(false),
    error = $state(''),
    pending = $state<{ id: string; files: number } | null>(null);
  const deleteLabel = $derived(
    dialog === 'replace'
      ? 'Delete and search again'
      : kind === 'show'
        ? 'Delete series files'
        : kind === 'season'
          ? 'Delete season files'
          : 'Delete files',
  );

  onMount(() => {
    if ((dialog !== 'delete' && dialog !== 'replace') || kept) return;
    preparing = true;
    prepareOperation(id, 'delete')
      .then((operation) => (pending = operation))
      .catch((caught) => (error = String(caught)))
      .finally(() => (preparing = false));
  });

  async function confirmDelete() {
    if (!pending) return;
    busy = true;
    error = '';
    const finish = startActivity({
      target: ['show', 'season', 'episode'].includes(kind) ? 'shows' : 'movies',
      resource: id,
      media_ids: [id],
      message: 'Deleting files',
    });
    try {
      await executeOperation(pending.id);
      pending = null;
      changed?.();
      await replace?.();
      close();
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
      finish();
      await refreshAttention();
    }
  }
</script>

{#if dialog === 'match'}
  <Modal title={`Fix match · ${title}`} onClose={close}>
    <MetadataEditor {id} {kind} {title} heading={false} />
  </Modal>
{:else if dialog === 'timestamps'}
  <Modal title={`Timestamps · ${title}`} onClose={close} size="workflow">
    {#each present as file (file.id)}<SegmentEditor
        mediaId={id}
        fileId={file.id}
        episode={kind === 'episode'}
        duration={Number(file.probe.format?.duration ?? 0)}
        summary={present.length > 1
          ? file.edition || 'Original edition'
          : 'Intro, recap, credits and preview'}
        open={present.length === 1}
      />{:else}<p class="text-xs text-muted">
        This title has no files to analyze.
      </p>{/each}
  </Modal>
{:else if dialog === 'mapping'}
  <Modal title={`Episode mapping · ${title}`} onClose={close}>
    <EpisodeMapping {id} changed={() => changed?.()} />
  </Modal>
{:else}
  <Modal title={`${deleteLabel} · ${title}`} {busy} onClose={close}>
    {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
    {#if kept}<p class="text-sm">
        These files can't be deleted while they are kept. {kept}
      </p>
    {:else if pending}
      <p class="text-sm">
        {#if dialog === 'replace'}Delete {pending.files}
          {pending.files === 1 ? 'file' : 'files'} from disk and search Sonarr for
          another release?{:else}Delete {pending.files}
          {pending.files === 1 ? 'file' : 'files'} from disk and stop Radarr or Sonarr
          from downloading {pending.files === 1 ? 'it' : 'them'} again?{/if}
      </p>
    {:else if preparing}<ContentSkeleton label="Checking files" />{/if}
    {#snippet actions()}
      <Button variant="secondary" size="form" disabled={busy} onclick={close}
        >Cancel</Button
      >
      <Button
        variant="danger"
        size="form"
        disabled={busy || preparing || !pending || !!kept}
        onclick={() => void confirmDelete()}>{deleteLabel}</Button
      >
    {/snippet}
  </Modal>
{/if}
