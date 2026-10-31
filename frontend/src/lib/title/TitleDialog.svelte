<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from '../ui/Modal.svelte';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import MetadataEditor from '../MetadataEditor.svelte';
  import SegmentEditor from '../SegmentEditor.svelte';
  import EpisodeMapping from '../EpisodeMapping.svelte';
  import { appearance } from '../appearance';
  import { startActivity } from '../activity';
  import { refreshAttention } from '../attention';
  import {
    executeOperation,
    prepareOperation,
    rememberedRadarrOptions,
    rememberRadarrOptions,
    removeMovie,
    type CatalogFile,
    type RadarrOptions,
  } from './catalog';

  export type TitleDialogKind =
    'match' | 'timestamps' | 'mapping' | 'delete' | 'replace' | 'remove';
  let {
    dialog,
    id,
    kind,
    title,
    files = [],
    kept = '',
    radarr = false,
    replace,
    close,
    changed,
  } = $props<{
    dialog: TitleDialogKind;
    /** A library id, or the `tmdb:` reference of a movie only Radarr lists. */
    id: string;
    kind: string;
    title: string;
    files?: CatalogFile[];
    /** Why Keep blocks deleting these files, if it does. */
    kept?: string;
    /** Whether Radarr lists this movie. */
    radarr?: boolean;
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
  const options = $derived(
    rememberedRadarrOptions($appearance.provider_preferences),
  );
  // Deleting a movie's files can also take it out of Radarr.
  const radarrChoices = $derived(
    dialog === 'delete' && kind === 'movie' && radarr,
  );
  const deleteLabel = $derived(
    dialog === 'replace'
      ? 'Delete and search again'
      : dialog === 'remove'
        ? 'Remove from Radarr'
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
    if (dialog !== 'remove' && !pending) return;
    busy = true;
    error = '';
    const finish = startActivity({
      target: ['show', 'season', 'episode'].includes(kind) ? 'shows' : 'movies',
      resource: id,
      media_ids: [id],
      message: dialog === 'remove' ? 'Removing from Radarr' : 'Deleting files',
    });
    try {
      if (dialog === 'remove') await removeMovie(id, options.exclude);
      else if (pending) {
        await executeOperation(pending.id, radarrChoices ? options : undefined);
        pending = null;
      }
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
        {dialog === 'remove'
          ? "This movie can't be removed from Radarr while it is kept."
          : "These files can't be deleted while they are kept."}
        {kept}
      </p>
    {:else if dialog === 'remove'}
      {@render choice("Add to Radarr's exclusion list", 'exclude')}
    {:else if pending}
      <p class="text-sm">
        {#if dialog === 'replace'}Delete {pending.files}
          {pending.files === 1 ? 'file' : 'files'} from disk and search Sonarr for
          another release?{:else}Delete {pending.files}
          {pending.files === 1 ? 'file' : 'files'} from disk and stop Radarr or Sonarr
          from downloading {pending.files === 1 ? 'it' : 'them'} again?{/if}
      </p>
      {#if radarrChoices}<div class="mt-4 grid gap-2.5">
          {@render choice('Remove from Radarr', 'remove')}
          {@render choice("Add to Radarr's exclusion list", 'exclude')}
        </div>{/if}
    {:else if preparing}<ContentSkeleton label="Checking files" />{/if}
    {#snippet actions()}
      <Button variant="secondary" size="form" disabled={busy} onclick={close}
        >Cancel</Button
      >
      <Button
        variant="danger"
        size="form"
        disabled={busy ||
          preparing ||
          (dialog !== 'remove' && !pending) ||
          !!kept}
        onclick={() => void confirmDelete()}>{deleteLabel}</Button
      >
    {/snippet}
  </Modal>
{/if}

{#snippet choice(label: string, key: keyof RadarrOptions)}<label
    class="flex w-fit cursor-pointer items-center gap-2 text-sm"
    ><input
      type="checkbox"
      class="accent-accent"
      checked={options[key]}
      disabled={busy}
      onchange={(event) =>
        rememberRadarrOptions({
          ...options,
          [key]: event.currentTarget.checked,
        })}
    />{label}</label
  >{/snippet}
