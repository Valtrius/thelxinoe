<script lang="ts">
  import { DropdownMenu } from 'bits-ui';
  import {
    Bookmark,
    BookmarkX,
    Check,
    Ellipsis,
    ExternalLink,
    Eye,
    EyeOff,
    Heart,
    HeartOff,
    ListVideo,
    LoaderCircle,
    Play,
    RefreshCw,
    Search,
    Timer,
    Trash2,
  } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import { startActivity } from '../activity';
  import { refreshAttention } from '../attention';
  import { showToast } from '../providers/toasts';
  import TitleDialog, { type TitleDialogKind } from './TitleDialog.svelte';
  import {
    executeOperation,
    loadFlags,
    loadItem,
    metadataNumber,
    prepareOperation,
    refreshMetadata,
    setFlag,
    setKeep,
    keptReason,
    type CatalogItem,
    type Flags,
  } from './catalog';

  let {
    id,
    kind,
    title,
    admin,
    variant = 'button',
    watchLater = true,
    favorite = true,
    tmdbLink = true,
    play,
    changed,
  } = $props<{
    id: string;
    kind: string;
    title: string;
    admin: boolean;
    /** card: poster overlay, button: page header, row: episode row */
    variant?: 'card' | 'button' | 'row';
    /** Omit actions the caller already shows as buttons. */
    watchLater?: boolean;
    favorite?: boolean;
    /** Title pages link TMDB themselves. */
    tmdbLink?: boolean;
    play?: (fileId: string) => void;
    changed?: () => void;
  }>();
  let open = $state(false),
    loading = $state(false),
    flags = $state<Flags | null>(null),
    item = $state<CatalogItem | null>(null),
    dialog = $state<TitleDialogKind | null>(null);
  // Dialogs open once the menu has closed and returned focus to its trigger, so
  // closing a dialog can return focus there too.
  let requested: TitleDialogKind | null = null,
    generation = 0;
  const present = $derived(item?.files?.filter((file) => file.present) ?? []);
  // A season or series' Keep covers this item; its own switch has no effect.
  const inherited = $derived(item?.kept_by ?? null);
  const kept = $derived(item ? keptReason(item) : '');
  const tmdb = $derived(metadataNumber(item, 'tmdb_id'));
  const playable = $derived(['movie', 'episode'].includes(kind));
  const manageable = $derived(['movie', 'show'].includes(kind));
  // A series page already shows its own Watch Later, Favorite and TMDB link.
  const empty = $derived(
    !admin &&
      !playable &&
      !watchLater &&
      !favorite &&
      !(tmdbLink && manageable) &&
      !play,
  );
  const target = $derived(
    ['show', 'season', 'episode'].includes(kind) ? 'shows' : 'movies',
  );
  const itemClass =
    'flex h-8 w-full cursor-default items-center gap-[0.55rem] whitespace-nowrap px-[0.55rem] py-[0.4rem] text-left text-[0.7rem] text-muted outline-none data-[disabled]:opacity-45 data-[highlighted]:bg-accent-soft data-[highlighted]:text-foreground [&_svg]:size-[0.85rem] [&_svg]:shrink-0 compact:h-11 compact:text-[13px]';
  const headingClass =
    'px-[0.55rem] pt-2 pb-1 text-[9px] font-semibold tracking-[0.12em] text-muted uppercase';

  async function load() {
    const request = ++generation;
    loading = true;
    try {
      const [nextFlags, nextItem] = await Promise.all([
        loadFlags(id),
        loadItem(id),
      ]);
      if (request !== generation) return;
      flags = nextFlags;
      item = nextItem;
    } catch (caught) {
      if (request !== generation) return;
      open = false;
      showToast({
        key: `title-menu:${id}`,
        tone: 'error',
        title,
        message: String(caught),
      });
    } finally {
      if (request === generation) loading = false;
    }
  }

  async function act(
    message: string,
    done: string,
    run: () => Promise<unknown>,
  ) {
    const finish = startActivity({
      target,
      resource: id,
      media_ids: [id],
      message,
    });
    try {
      await run();
      showToast({
        key: `title-menu:${id}`,
        tone: 'success',
        title,
        message: done,
      });
      changed?.();
    } catch (caught) {
      showToast({
        key: `title-menu:${id}`,
        tone: 'error',
        title,
        message: String(caught),
      });
    } finally {
      finish();
      await refreshAttention();
    }
  }

  function toggle(key: keyof Flags, done: string) {
    if (!flags) return;
    const value = !flags[key];
    void act('Saving', done, async () => {
      flags = await setFlag(id, key, value);
    });
  }

  function monitor(value: boolean) {
    void act(
      value ? 'Monitoring files' : 'Unmonitoring files',
      value ? 'Radarr or Sonarr monitors it again.' : 'No longer monitored.',
      async () => {
        const operation = await prepareOperation(
          id,
          value ? 'monitor' : 'unmonitor',
        );
        await executeOperation(operation.id);
      },
    );
  }
</script>

{#if !empty}<DropdownMenu.Root
    bind:open
    onOpenChange={(value) => {
      if (value) void load();
    }}
  >
    <DropdownMenu.Trigger>
      {#snippet child({ props })}
        {#if variant === 'card'}<button
            {...props}
            class="grid size-7.5 min-h-0 place-items-center border border-[#ffffff30] bg-[#080b10c9] p-0 text-white"
            title="More actions"
            aria-label={`More actions for ${title}`}
            ><Ellipsis size={16} /></button
          >{:else}<Button
            {...props}
            variant={variant === 'row' ? 'ghost' : 'secondary'}
            size={variant === 'row' ? 'compact-icon' : 'icon'}
            title="More actions"
            aria-label={`More actions for ${title}`}
            ><Ellipsis size={16} /></Button
          >{/if}
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.Portal>
      <DropdownMenu.Content
        align="end"
        sideOffset={4}
        collisionPadding={8}
        preventScroll={false}
        class="z-50 grid max-h-[min(30rem,calc(100dvh-1rem))] min-w-52 overflow-y-auto border border-line-strong bg-surface-strong p-1.5 shadow-2xl"
        aria-label={`Actions for ${title}`}
        onCloseAutoFocus={() => {
          const next = requested;
          requested = null;
          if (next) setTimeout(() => (dialog = next));
        }}
      >
        {#if loading || !flags}
          <div class="{itemClass} pointer-events-none" aria-hidden="true">
            <LoaderCircle
              class="animate-spin motion-reduce:animate-none"
            />Loading
          </div>
        {:else}
          {#if playable}
            <DropdownMenu.Item
              class={itemClass}
              onSelect={() =>
                toggle(
                  'watched',
                  flags!.watched ? 'Marked unwatched.' : 'Marked watched.',
                )}
              >{#if flags.watched}<EyeOff />Mark unwatched{:else}<Eye />Mark
                watched{/if}</DropdownMenu.Item
            >
          {/if}
          {#if watchLater}
            <DropdownMenu.Item
              class={itemClass}
              onSelect={() =>
                toggle(
                  'watch_later',
                  flags!.watch_later
                    ? 'Removed from Watch Later.'
                    : 'Added to Watch Later.',
                )}
              >{#if flags.watch_later}<BookmarkX />Remove from Watch Later{:else}<Bookmark
                />Add to Watch Later{/if}</DropdownMenu.Item
            >
          {/if}
          {#if favorite}
            <DropdownMenu.Item
              class={itemClass}
              onSelect={() =>
                toggle(
                  'favorite',
                  flags!.favorite
                    ? 'Removed from Favorites.'
                    : 'Added to Favorites.',
                )}
              >{#if flags.favorite}<HeartOff />Remove from Favorites{:else}<Heart
                />Add to Favorites{/if}</DropdownMenu.Item
            >
          {/if}
          {#if play && present.length > 1}
            <DropdownMenu.Sub>
              <DropdownMenu.SubTrigger class={itemClass}
                ><Play />Play edition</DropdownMenu.SubTrigger
              >
              <DropdownMenu.SubContent
                class="z-50 grid min-w-44 border border-line-strong bg-surface-strong p-1.5 shadow-2xl"
              >
                {#each present as file (file.id)}<DropdownMenu.Item
                    class={itemClass}
                    onSelect={() => play(file.id)}
                    >{file.edition || 'Original edition'}</DropdownMenu.Item
                  >{/each}
              </DropdownMenu.SubContent>
            </DropdownMenu.Sub>
          {/if}
          {#if tmdbLink && tmdb && manageable}
            <DropdownMenu.Item class={itemClass}>
              {#snippet child({ props })}<a
                  {...props}
                  href={`https://www.themoviedb.org/${kind === 'show' ? 'tv' : 'movie'}/${tmdb}`}
                  target="_blank"
                  rel="noreferrer"><ExternalLink />Open on TMDB</a
                >{/snippet}
            </DropdownMenu.Item>
          {/if}
          {#if admin && item}
            <DropdownMenu.Separator class="my-1 h-px bg-line" />
            <DropdownMenu.Group>
              <DropdownMenu.GroupHeading class={headingClass}
                >Administration</DropdownMenu.GroupHeading
              >
              {#if manageable}
                <DropdownMenu.Item
                  class={itemClass}
                  onSelect={() => (requested = 'match')}
                  ><Search />Fix match…</DropdownMenu.Item
                >
                <DropdownMenu.Item
                  class={itemClass}
                  onSelect={() =>
                    void act(
                      'Refreshing metadata',
                      'Metadata refresh queued.',
                      () => refreshMetadata(id),
                    )}><RefreshCw />Refresh metadata</DropdownMenu.Item
                >
              {/if}
              {#if playable && present.length}
                <DropdownMenu.Item
                  class={itemClass}
                  onSelect={() => (requested = 'timestamps')}
                  ><Timer />Edit timestamps…</DropdownMenu.Item
                >
              {/if}
              {#if kind === 'episode'}
                <DropdownMenu.Item
                  class={itemClass}
                  onSelect={() => (requested = 'mapping')}
                  ><ListVideo />Episode mapping…</DropdownMenu.Item
                >
              {/if}
              <DropdownMenu.CheckboxItem
                class={itemClass}
                checked={(item.keep ?? false) || !!inherited}
                disabled={!!inherited}
                title={inherited ? kept : undefined}
                onCheckedChange={(value) =>
                  void act(
                    'Saving',
                    value
                      ? 'Kept: automatic cleanup skips it.'
                      : 'Automatic cleanup may remove it again.',
                    () => setKeep(id, value),
                  )}
                >{#snippet children({ checked })}<span
                    class="grid size-[0.85rem] place-items-center"
                    >{#if checked}<Check class="text-accent" />{/if}</span
                  >Keep{#if inherited}<span class="ml-auto pl-3 text-[0.65rem]"
                      >with the {inherited === 'show'
                        ? 'series'
                        : 'season'}</span
                    >{:else if item?.kept_within}<span
                      class="ml-auto pl-3 text-[0.65rem]"
                      >{item?.kept_within} inside</span
                    >{/if}{/snippet}</DropdownMenu.CheckboxItem
              >
              {#if item.status?.monitored !== undefined}
                <DropdownMenu.CheckboxItem
                  class={itemClass}
                  checked={item.status.monitored}
                  disabled={!!kept}
                  title={kept || undefined}
                  onCheckedChange={(value) => monitor(value)}
                  >{#snippet children({ checked })}<span
                      class="grid size-[0.85rem] place-items-center"
                      >{#if checked}<Check class="text-accent" />{/if}</span
                    >Monitored{#if kept}<span
                        class="ml-auto pl-3 text-[0.65rem]">kept</span
                      >{/if}{/snippet}</DropdownMenu.CheckboxItem
                >
              {/if}
              <DropdownMenu.Item
                class="{itemClass} text-danger! data-[highlighted]:bg-danger/12"
                onSelect={() => (requested = 'delete')}
                ><Trash2 />{kind === 'show'
                  ? 'Delete series files…'
                  : kind === 'season'
                    ? 'Delete season files…'
                    : 'Delete files…'}</DropdownMenu.Item
              >
            </DropdownMenu.Group>
          {/if}
        {/if}
      </DropdownMenu.Content>
    </DropdownMenu.Portal>
  </DropdownMenu.Root>{/if}
{#if dialog && item}<TitleDialog
    {dialog}
    {id}
    {kind}
    {title}
    files={item.files}
    {kept}
    close={() => (dialog = null)}
    {changed}
  />{/if}
