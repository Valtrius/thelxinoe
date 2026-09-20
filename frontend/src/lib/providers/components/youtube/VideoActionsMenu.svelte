<script lang="ts">
  import type { YoutubeVideoActions } from '../../youtube-video-actions';

  import { tick } from 'svelte';
  import {
    Copy,
    Check,
    Download,
    Ellipsis,
    Eye,
    EyeOff,
    ExternalLink,
    ListMinus,
    ListPlus,
    LogIn,
    Pin,
    PinOff,
    RotateCcw,
    Settings2,
    Trash2,
    X,
  } from '@lucide/svelte';
  import type {
    YoutubeCardShortcut,
    YoutubeWatchlist,
    YoutubeVideo,
  } from '../../types';

  import {
    MAX_YOUTUBE_CARD_SHORTCUTS,
    toggleYoutubeCardShortcut,
    youtubeCardShortcutOptions,
  } from '../../youtube-card-shortcuts';
  import CardShortcutButton from '../ui/CardShortcutButton.svelte';
  import type { YoutubeVideoPresentation } from '../../youtube-video-presentation';
  let {
    presentation,
    watchlists,
    video,
    onMenuToggle,
    youtubeCardShortcuts,
    actions,
    onShortcutDeleteRequest,
    onShortcutDeleteCancel,
    signedInPlaybackAvailable,
    downloadBusy,
    isPlaying,
    menuOpen,
    shortcutDeleteConfirming,
    ytdlpReady,
    ffmpegReady,
  }: {
    presentation: YoutubeVideoPresentation;
    actions: YoutubeVideoActions;
    video: YoutubeVideo;
    ytdlpReady: boolean;
    ffmpegReady: boolean;
    youtubeCardShortcuts: YoutubeCardShortcut[];
    signedInPlaybackAvailable: boolean;
    isPlaying: boolean;
    downloadBusy: boolean;
    watchlists: YoutubeWatchlist[];
    menuOpen: boolean;
    shortcutDeleteConfirming: boolean;
    onMenuToggle: () => void;
    onShortcutDeleteRequest: () => void;
    onShortcutDeleteCancel: () => void;
  } = $props();
  const activeDownload = $derived(presentation.activeDownload);
  const downloadReady = $derived(presentation.downloadReady);
  const downloadPending = $derived(presentation.downloadPending);
  const canPlay = $derived(presentation.canPlay);
  const canDownload = $derived(presentation.canDownload);
  const isBusy = $derived(presentation.isBusy);
  const tracksVideoProgress = $derived(presentation.tracksVideoProgress);
  const downloadLabel = $derived(presentation.downloadLabel);
  const defaultWatchlist = $derived(
    watchlists.find((watchlist) => watchlist.isDefault),
  );

  const inDefaultWatchlist = $derived(
    defaultWatchlist?.items.some(
      (item) => item.video.videoId === video.videoId,
    ) ?? false,
  );

  let menuButton = $state<HTMLButtonElement | null>(null);

  let menuElement = $state<HTMLDivElement | null>(null);

  let menuLeft = $state(0);

  let menuTop = $state(0);

  let menuPlaced = $state(false);

  let choosingShortcuts = $state(false);

  let confirmingMenuDelete = $state(false);

  let draftShortcuts = $state<YoutubeCardShortcut[]>([]);

  let menuDeleteConfirmButton = $state<HTMLButtonElement | null>(null);

  let shortcutDeleteCancelButton = $state<HTMLButtonElement | null>(null);

  const menuGap = 4;

  const menuViewportMargin = 8;

  const menuItemClass =
    'flex h-8 w-full items-center gap-[0.55rem] whitespace-nowrap border-0 bg-transparent px-[0.55rem] py-[0.4rem] text-left text-[0.68rem] text-(--muted) enabled:hover:bg-(--accent-soft) enabled:hover:text-(--foreground) [&_svg]:size-[0.85rem] [&_svg]:shrink-0';

  function run(action: (video: YoutubeVideo) => void) {
    onMenuToggle();
    action(video);
  }

  async function beginChoosingShortcuts() {
    draftShortcuts = [...youtubeCardShortcuts];
    choosingShortcuts = true;
    await tick();
    menuElement
      ?.querySelector<HTMLButtonElement>('[data-shortcut-option]')
      ?.focus();
  }

  function toggleShortcut(shortcut: YoutubeCardShortcut) {
    draftShortcuts = toggleYoutubeCardShortcut(draftShortcuts, shortcut);
  }

  function saveShortcuts(event: MouseEvent) {
    choosingShortcuts = false;
    onMenuToggle();
    if (event.detail === 0) void tick().then(() => menuButton?.focus());
    if (
      draftShortcuts.length !== youtubeCardShortcuts.length ||
      draftShortcuts.some(
        (shortcut, index) => shortcut !== youtubeCardShortcuts[index],
      )
    ) {
      actions.onYoutubeCardShortcutsChanged([...draftShortcuts]);
    }
  }

  async function beginMenuDelete(event: MouseEvent) {
    confirmingMenuDelete = true;
    if (event.detail === 0) {
      await tick();
      menuDeleteConfirmButton?.focus();
    }
  }

  function confirmMenuDelete(event: MouseEvent) {
    if (event.detail > 0) {
      (event.currentTarget as HTMLButtonElement).blur();
    }
    confirmingMenuDelete = false;
    run(actions.onDeleteDownload);
  }

  function cancelMenuDelete(event: MouseEvent) {
    if (event.detail > 0) {
      (event.currentTarget as HTMLButtonElement).blur();
    }
    confirmingMenuDelete = false;
  }

  async function requestShortcutDelete(event: MouseEvent) {
    if (event.detail > 0) {
      (event.currentTarget as HTMLButtonElement).blur();
    }
    onShortcutDeleteRequest();
    if (event.detail === 0) {
      await tick();
      shortcutDeleteCancelButton?.focus();
    }
  }

  function cancelShortcutDelete(event: MouseEvent) {
    if (event.detail > 0) {
      (event.currentTarget as HTMLButtonElement).blur();
    }
    onShortcutDeleteCancel();
  }

  function confirmShortcutDelete(event: MouseEvent) {
    if (event.detail > 0) {
      (event.currentTarget as HTMLButtonElement).blur();
    }
    onShortcutDeleteCancel();
    actions.onDeleteDownload(video);
  }

  function shortcutAvailable(shortcut: YoutubeCardShortcut) {
    switch (shortcut) {
      case 'play_signed_in':
        return signedInPlaybackAvailable && !downloadReady;
      case 'download':
        return tracksVideoProgress && !downloadReady;
      case 'play_from_beginning':
      case 'watch_toggle':
        return tracksVideoProgress;
      case 'pin_download':
        return downloadReady;
      case 'delete_download':
        return Boolean(activeDownload);
      case 'add_watch_later':
        return Boolean(defaultWatchlist);
      default:
        return true;
    }
  }

  function shortcutDisabled(shortcut: YoutubeCardShortcut) {
    switch (shortcut) {
      case 'play_from_beginning':
      case 'play_signed_in':
        return !canPlay;
      case 'download':
        return !canDownload || downloadPending;
      case 'watch_toggle':
        return !tracksVideoProgress || isBusy;
      case 'pin_download':
        return downloadBusy;
      case 'delete_download':
        return downloadBusy || isPlaying;
      case 'add_watch_later':
        return !defaultWatchlist;
      default:
        return false;
    }
  }

  function shortcutLabel(shortcut: YoutubeCardShortcut) {
    switch (shortcut) {
      case 'play_from_beginning':
        return 'Play from beginning';
      case 'play_signed_in':
        return 'Play signed in';
      case 'download':
        return downloadPending
          ? (downloadLabel ?? 'Download in progress')
          : 'Download';
      case 'watch_toggle':
        return video.isWatched ? 'Mark unwatched' : 'Mark watched';
      case 'copy_url':
        return 'Copy YouTube URL';
      case 'open_browser':
        return 'Open in browser';
      case 'pin_download':
        return activeDownload?.pinned ? 'Unpin download' : 'Pin download';
      case 'delete_download':
        return downloadReady ? 'Delete download' : 'Delete partial download';
      case 'add_watch_later':
        return inDefaultWatchlist
          ? 'Remove from Watch Later'
          : 'Add to Watch Later';
    }
  }

  function runShortcut(shortcut: YoutubeCardShortcut, event: MouseEvent) {
    if (event.detail > 0) {
      (event.currentTarget as HTMLButtonElement).blur();
    }
    if (menuOpen) onMenuToggle();
    switch (shortcut) {
      case 'play_from_beginning':
        actions.onBeginning(video);
        break;
      case 'play_signed_in':
        actions.onSignedIn(video);
        break;
      case 'download':
        actions.onDownload(video);
        break;
      case 'watch_toggle':
        (video.isWatched ? actions.onMarkUnwatched : actions.onMarkWatched)(
          video,
        );
        break;
      case 'copy_url':
        actions.onCopy(video);
        break;
      case 'open_browser':
        actions.onOpen(video);
        break;
      case 'pin_download':
        (activeDownload?.pinned
          ? actions.onUnpinDownload
          : actions.onPinDownload)(video);
        break;
      case 'delete_download':
        void requestShortcutDelete(event);
        break;
      case 'add_watch_later':
        if (defaultWatchlist) {
          (inDefaultWatchlist
            ? actions.onRemoveFromWatchlist
            : actions.onAddToWatchlist)(video, defaultWatchlist.id);
        }
        break;
    }
  }

  function positionMenu() {
    if (!menuOpen || !menuButton || !menuElement?.isConnected) return;
    if (!menuElement.matches(':popover-open')) menuElement.showPopover();

    const triggerBounds = menuButton.getBoundingClientRect();
    const menuBounds = menuElement.getBoundingClientRect();
    const spaceAbove = triggerBounds.top - menuViewportMargin;
    const spaceBelow =
      window.innerHeight - triggerBounds.bottom - menuViewportMargin;
    const opensUp = menuBounds.height > spaceBelow && spaceAbove > spaceBelow;
    const preferredTop = opensUp
      ? triggerBounds.top - menuBounds.height - menuGap
      : triggerBounds.bottom + menuGap;
    const maximumTop = Math.max(
      menuViewportMargin,
      window.innerHeight - menuBounds.height - menuViewportMargin,
    );
    const maximumLeft = Math.max(
      menuViewportMargin,
      window.innerWidth - menuBounds.width - menuViewportMargin,
    );

    menuTop = Math.round(
      Math.min(Math.max(menuViewportMargin, preferredTop), maximumTop),
    );
    menuLeft = Math.round(
      Math.min(
        Math.max(menuViewportMargin, triggerBounds.right - menuBounds.width),
        maximumLeft,
      ),
    );
    menuPlaced = true;
  }

  $effect(() => {
    if (!menuOpen) {
      choosingShortcuts = false;
      confirmingMenuDelete = false;
      menuPlaced = false;
      if (menuElement?.matches(':popover-open')) menuElement.hidePopover();
      return;
    }

    let stopped = false;
    let resizeObserver: ResizeObserver | null = null;
    const reposition = () => positionMenu();
    void tick().then(() => {
      if (stopped) return;
      positionMenu();
      if (menuElement) {
        resizeObserver = new ResizeObserver(reposition);
        resizeObserver.observe(menuElement);
      }
    });
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);

    return () => {
      stopped = true;
      resizeObserver?.disconnect();
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
      if (menuElement?.matches(':popover-open')) menuElement.hidePopover();
      menuPlaced = false;
    };
  });

  $effect(() => {
    if (shortcutDeleteConfirming && !activeDownload) {
      onShortcutDeleteCancel();
    }
  });
</script>

{#snippet shortcutIcon(shortcut: YoutubeCardShortcut)}
  {#if shortcut === 'play_from_beginning'}
    <RotateCcw />
  {:else if shortcut === 'play_signed_in'}
    <LogIn />
  {:else if shortcut === 'download'}
    <Download />
  {:else if shortcut === 'watch_toggle'}
    {#if video.isWatched}<EyeOff />{:else}<Eye />{/if}
  {:else if shortcut === 'copy_url'}
    <Copy />
  {:else if shortcut === 'open_browser'}
    <ExternalLink />
  {:else if shortcut === 'pin_download'}
    {#if activeDownload?.pinned}<PinOff />{:else}<Pin />{/if}
  {:else if shortcut === 'add_watch_later'}
    {#if inDefaultWatchlist}<ListMinus />{:else}<ListPlus />{/if}
  {:else}
    <Trash2 />
  {/if}
{/snippet}

{#snippet deleteDownloadMenuItem(label: string, disabled: boolean)}
  {#if confirmingMenuDelete}
    <div
      class="grid min-w-full grid-cols-2 gap-1"
      role="group"
      aria-label={`${label} confirmation`}
    >
      <button
        bind:this={menuDeleteConfirmButton}
        type="button"
        class="grid h-8 place-items-center border border-[color-mix(in_srgb,var(--success)_55%,var(--line-strong))] bg-[color-mix(in_srgb,var(--success)_12%,transparent)] text-(--success) hover:bg-[color-mix(in_srgb,var(--success)_20%,transparent)] [&_svg]:size-[0.9rem]"
        title={`Confirm ${label.toLowerCase()}`}
        aria-label={`Confirm ${label.toLowerCase()}`}
        onclick={confirmMenuDelete}
      >
        <Check />
      </button>
      <button
        type="button"
        class="grid h-8 place-items-center border border-[color-mix(in_srgb,var(--danger)_55%,var(--line-strong))] bg-[color-mix(in_srgb,var(--danger)_12%,transparent)] text-(--danger) hover:bg-[color-mix(in_srgb,var(--danger)_20%,transparent)] [&_svg]:size-[0.9rem]"
        title={`Cancel ${label.toLowerCase()}`}
        aria-label={`Cancel ${label.toLowerCase()}`}
        onclick={cancelMenuDelete}
      >
        <X />
      </button>
    </div>
  {:else}
    <button
      class={menuItemClass}
      type="button"
      {disabled}
      onclick={beginMenuDelete}
    >
      <Trash2 />{label}
    </button>
  {/if}
{/snippet}

{#if menuOpen}
  <button
    type="button"
    class="absolute inset-0 z-40 cursor-default bg-transparent"
    aria-label="Close video actions"
    onclick={onMenuToggle}
  ></button>
{/if}

<div
  class={`absolute top-2 right-2 z-50 flex items-center gap-1 transition-opacity ${menuOpen || shortcutDeleteConfirming ? 'pointer-events-auto opacity-100' : 'pointer-events-none opacity-0 group-focus-within:pointer-events-auto group-focus-within:opacity-100 group-hover:pointer-events-auto group-hover:opacity-100'}`}
>
  {#each youtubeCardShortcuts as shortcut (shortcut)}
    {#if shortcutAvailable(shortcut)}
      {#if shortcut === 'delete_download' && shortcutDeleteConfirming}
        <button
          bind:this={shortcutDeleteCancelButton}
          type="button"
          class="grid size-8 place-items-center border border-[color-mix(in_srgb,var(--danger)_60%,var(--line-strong))] bg-[color-mix(in_srgb,var(--danger)_14%,var(--background))] text-(--danger) hover:bg-[color-mix(in_srgb,var(--danger)_22%,var(--background))] [&_svg]:size-4"
          title="Cancel deleting download"
          aria-label={`Cancel deleting download: ${video.title}`}
          onclick={cancelShortcutDelete}
        >
          <X />
        </button>
        <button
          type="button"
          class="grid size-8 place-items-center border border-[color-mix(in_srgb,var(--success)_60%,var(--line-strong))] bg-[color-mix(in_srgb,var(--success)_14%,var(--background))] text-(--success) hover:bg-[color-mix(in_srgb,var(--success)_22%,var(--background))] [&_svg]:size-4"
          title="Confirm deleting download"
          aria-label={`Confirm deleting download: ${video.title}`}
          onclick={confirmShortcutDelete}
        >
          <Check />
        </button>
      {:else}
        <CardShortcutButton
          platform="youtube"
          disabled={shortcutDisabled(shortcut)}
          title={shortcutLabel(shortcut)}
          aria-label={`${shortcutLabel(shortcut)}: ${video.title}`}
          onclick={(event) => runShortcut(shortcut, event)}
        >
          {@render shortcutIcon(shortcut)}
        </CardShortcutButton>
      {/if}
    {/if}
  {/each}
  <CardShortcutButton
    bind:element={menuButton}
    platform="youtube"
    aria-label={`More actions for ${video.title}`}
    aria-expanded={menuOpen}
    aria-controls={`video-actions-${video.videoId}`}
    onclick={onMenuToggle}
  >
    <Ellipsis class="size-4" />
  </CardShortcutButton>
  {#if menuOpen}
    <div
      bind:this={menuElement}
      id={`video-actions-${video.videoId}`}
      popover="manual"
      class="media-card-popover panel fixed right-auto bottom-auto m-0 grid max-h-[calc(100vh-1rem)] w-max max-w-[calc(100vw-1rem)] overflow-auto border border-(--line-strong) bg-(--surface-strong) p-1.5 shadow-2xl"
      style={`top:${menuTop}px;left:${menuLeft}px;visibility:${menuPlaced ? 'visible' : 'hidden'}`}
    >
      {#if choosingShortcuts}
        <div class="px-[0.55rem] pt-1 pb-2">
          <p class="text-xs font-semibold text-(--foreground)">
            Choose shortcuts
          </p>
          <p class="mt-1 text-[0.65rem] text-(--muted)">
            Select up to {MAX_YOUTUBE_CARD_SHORTCUTS}. Some only appear when
            they apply.
          </p>
        </div>
        {#each youtubeCardShortcutOptions as option (option.id)}
          {@const selected = draftShortcuts.includes(option.id)}
          <button
            class={menuItemClass}
            data-shortcut-option
            type="button"
            aria-pressed={selected}
            disabled={!selected &&
              draftShortcuts.length >= MAX_YOUTUBE_CARD_SHORTCUTS}
            onclick={() => toggleShortcut(option.id)}
          >
            {@render shortcutIcon(option.id)}
            <span>{option.label}</span>
            {#if selected}<Check class="ml-auto text-(--accent)" />{/if}
          </button>
        {/each}
        <span class="my-1 h-px bg-(--line)"></span>
        <button class={menuItemClass} type="button" onclick={saveShortcuts}>
          <Check />Done
          <span class="ml-auto text-[0.6rem] text-(--muted)"
            >{draftShortcuts.length}/{MAX_YOUTUBE_CARD_SHORTCUTS}</span
          >
        </button>
      {:else}
        {#if tracksVideoProgress}
          <button
            class={menuItemClass}
            type="button"
            disabled={!canPlay}
            onclick={() => run(actions.onBeginning)}
            ><RotateCcw />Play from beginning</button
          >
        {/if}
        {#if signedInPlaybackAvailable && !downloadReady}
          <button
            class={menuItemClass}
            type="button"
            disabled={!canPlay}
            onclick={() => run(actions.onSignedIn)}
            ><LogIn />Play signed in</button
          >
        {/if}
        {#if tracksVideoProgress}<span class="my-1 h-px bg-(--line)"
          ></span>{/if}
        {#if downloadReady}
          <button
            class={menuItemClass}
            type="button"
            disabled={downloadBusy}
            onclick={() =>
              run(
                activeDownload?.pinned
                  ? actions.onUnpinDownload
                  : actions.onPinDownload,
              )}
            >{#if activeDownload?.pinned}<PinOff />Unpin download{:else}<Pin
              />Pin download{/if}</button
          >
          {@render deleteDownloadMenuItem(
            'Delete download',
            downloadBusy || isPlaying,
          )}
        {:else if downloadPending}
          <button
            class={menuItemClass}
            type="button"
            disabled={downloadBusy}
            onclick={() => run(actions.onCancelDownload)}
            ><X />Cancel download</button
          >
        {:else if tracksVideoProgress}
          <button
            class={menuItemClass}
            type="button"
            disabled={!canDownload}
            title={!ytdlpReady
              ? 'Server extraction tools must be available.'
              : !ffmpegReady
                ? 'Downloads must be enabled by the administrator.'
                : undefined}
            onclick={() => run(actions.onDownload)}><Download />Download</button
          >
          {#if signedInPlaybackAvailable}
            <button
              class={menuItemClass}
              type="button"
              disabled={!canDownload}
              title={!ytdlpReady
                ? 'Server extraction tools must be available.'
                : !ffmpegReady
                  ? 'Downloads must be enabled by the administrator.'
                  : undefined}
              onclick={() => run(actions.onDownloadSignedIn)}
              ><LogIn />Download signed in</button
            >
          {/if}
          {#if activeDownload}
            {@render deleteDownloadMenuItem(
              'Delete partial download',
              downloadBusy,
            )}
          {/if}
        {/if}
        {#if tracksVideoProgress}
          <button
            class={menuItemClass}
            type="button"
            disabled={isBusy}
            onclick={() =>
              run(
                video.isWatched
                  ? actions.onMarkUnwatched
                  : actions.onMarkWatched,
              )}
            >{#if video.isWatched}<EyeOff />Mark unwatched{:else}<Eye />Mark
              watched{/if}</button
          >
        {/if}
        {#if watchlists.length > 0}
          <span class="my-1 h-px bg-(--line)"></span>
          {#each watchlists as watchlist (watchlist.id)}
            {@const alreadyAdded = watchlist.items.some(
              (item) => item.video.videoId === video.videoId,
            )}
            <button
              class={menuItemClass}
              type="button"
              disabled={alreadyAdded && !watchlist.isDefault}
              title={alreadyAdded && !watchlist.isDefault
                ? `Already in ${watchlist.name}`
                : undefined}
              onclick={() => {
                if (alreadyAdded && watchlist.isDefault) {
                  run((item) =>
                    actions.onRemoveFromWatchlist(item, watchlist.id),
                  );
                } else {
                  run((item) => actions.onAddToWatchlist(item, watchlist.id));
                }
              }}
            >
              {#if alreadyAdded && watchlist.isDefault}
                <ListMinus />Remove from {watchlist.name}
              {:else}
                {#if alreadyAdded}<Check />{:else}<ListPlus />{/if}
                {alreadyAdded ? 'In' : 'Add to'}
                {watchlist.name}
              {/if}
            </button>
          {/each}
        {/if}
        <span class="my-1 h-px bg-(--line)"></span>
        <button
          class={menuItemClass}
          type="button"
          onclick={() => run(actions.onCopy)}><Copy />Copy YouTube URL</button
        >
        <button
          class={menuItemClass}
          type="button"
          onclick={() => run(actions.onOpen)}
          ><ExternalLink />Open in browser</button
        >
        <span class="my-1 h-px bg-(--line)"></span>
        <button
          class={menuItemClass}
          type="button"
          onclick={beginChoosingShortcuts}
        >
          <Settings2 />Choose shortcuts…
          <span class="ml-auto text-[0.6rem] text-(--muted)"
            >{youtubeCardShortcuts.length}/{MAX_YOUTUBE_CARD_SHORTCUTS}</span
          >
        </button>
      {/if}
    </div>
  {/if}
</div>
