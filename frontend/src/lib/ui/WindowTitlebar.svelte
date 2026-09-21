<script lang="ts">
  import { onMount } from 'svelte';
  import { Copy, Minus, Square, X } from '@lucide/svelte';
  import { getVersion } from '@tauri-apps/api/app';
  import { isTauri } from '@tauri-apps/api/core';
  import {
    getCurrentWindow,
    type Window as TauriWindow,
  } from '@tauri-apps/api/window';
  const appIconUrl = '/icon.svg';
  import ThemeControls from './ThemeControls.svelte';

  let maximized = $state(false);
  let appVersion = $state<string | null>(null);
  let appWindow: TauriWindow | null = null;

  onMount(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    appWindow = getCurrentWindow();
    void (async () => {
      const [version, currentMaximized] = await Promise.all([
        getVersion(),
        appWindow?.isMaximized(),
      ]);
      if (disposed) return;
      appVersion = version;
      maximized = currentMaximized ?? false;
      unlisten = await appWindow?.onResized(async () => {
        if (!disposed && appWindow) maximized = await appWindow.isMaximized();
      });
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  function minimize() {
    void appWindow?.minimize();
  }

  function toggleMaximize() {
    void appWindow?.toggleMaximize();
  }

  function startWindowDrag(event: MouseEvent) {
    if (event.button !== 0) return;
    if (event.target instanceof Element && event.target.closest('button'))
      return;
    if (event.detail === 2) {
      toggleMaximize();
      return;
    }
    void appWindow?.startDragging();
  }

  function draggable(node: HTMLElement) {
    node.addEventListener('mousedown', startWindowDrag);
    return {
      destroy() {
        node.removeEventListener('mousedown', startWindowDrag);
      },
    };
  }

  function close() {
    void appWindow?.close();
  }
</script>

<header
  data-window-titlebar
  data-tauri-drag-region
  use:draggable
  class="fixed inset-x-0 top-0 z-200 flex h-8 items-center border-b border-line bg-background/94 pl-3 backdrop-blur-xl select-none [&_button]:focus-visible:-outline-offset-3"
>
  <div
    data-tauri-drag-region
    class="pointer-events-none flex min-w-0 items-center gap-2"
  >
    <img class="size-4 shrink-0" src={appIconUrl} alt="" aria-hidden="true" />
    <span
      class="truncate text-[0.62rem] font-semibold tracking-[0.15em] text-muted uppercase"
      >Thelxinoe
      {#if appVersion}
        · {appVersion}
      {/if}
    </span>
  </div>
  <div class="ml-auto flex h-full shrink-0 items-center">
    <ThemeControls />
    <span aria-hidden="true" class="mx-2 h-4 w-px bg-line"></span>
    <div class="flex h-full" aria-label="Window controls">
      <button
        type="button"
        class="grid h-full w-11 place-items-center text-muted hover:bg-surface-soft hover:text-foreground"
        aria-label="Minimize window"
        onclick={minimize}><Minus class="size-3.5" /></button
      >
      <button
        type="button"
        class="grid h-full w-11 place-items-center text-muted hover:bg-surface-soft hover:text-foreground"
        aria-label={maximized ? 'Restore window' : 'Maximize window'}
        onclick={toggleMaximize}
        >{#if maximized}<Copy class="size-3" />{:else}<Square
            class="size-3"
          />{/if}</button
      >
      <button
        type="button"
        class="grid h-full w-11 place-items-center text-muted hover:bg-danger hover:text-white"
        aria-label="Close window"
        onclick={close}><X class="size-3.5" /></button
      >
    </div>
  </div>
</header>
