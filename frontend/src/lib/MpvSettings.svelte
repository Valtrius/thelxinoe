<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  type Settings = {
    selection: { mode: string; path: string; version: string };
    configuration: string;
    text: string;
    plugins: string[];
    product_version: string;
  };
  let settings = $state<Settings | null>(null),
    path = $state(''),
    text = $state(''),
    error = $state(''),
    busy = $state(false),
    saved = $state(''),
    pluginName = $state(''),
    pluginText = $state('');
  async function load() {
    settings = await invoke<Settings>('mpv_settings');
    path = settings.selection.path;
    text = settings.text;
  }
  async function act(action: () => Promise<unknown>, message = 'Saved') {
    busy = true;
    error = '';
    saved = '';
    try {
      await action();
      await load();
      saved = message;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  onMount(() => {
    void act(load, '');
  });
</script>

<section class="panel">
  <h2>Windows player</h2>
  <p class="muted">
    Play video in MPV and control music here. MPV uses your Thelxinoe player
    configuration.
  </p>
  {#if error}<p role="alert" class="error">{error}</p>{/if}{#if saved}<p
      role="status"
    >
      {saved}
    </p>{/if}
  {#if settings}<div class="row">
      <div>
        <strong>{settings.selection.version || 'MPV is not configured'}</strong
        ><small
          >{settings.selection.mode === 'custom'
            ? 'Your installation'
            : 'Managed by Thelxinoe'}</small
        >
      </div>
      <button
        class="primary"
        disabled={busy}
        onclick={() =>
          void act(() => invoke('mpv_install'), 'MPV installed and verified')}
        >{busy ? 'Working…' : 'Install or update MPV'}</button
      >
    </div>
    <form
      class="inline-form"
      onsubmit={(e) => {
        e.preventDefault();
        void act(() => invoke('mpv_custom', { path }));
      }}
    >
      <label
        >Use my MPV installation<input
          bind:value={path}
          placeholder="C:\Tools\mpv\mpv.exe"
          required
        /></label
      ><button class="secondary" disabled={busy}>Use this executable</button>
    </form>
    <details>
      <summary>Player configuration and plugins</summary>
      <p class="muted">Configuration folder: {settings.configuration}</p>
      <label
        >mpv.conf<textarea
          rows="8"
          bind:value={text}
          spellcheck={false}
          placeholder="volume=80"></textarea></label
      ><button
        class="secondary"
        disabled={busy}
        onclick={() => void act(() => invoke('mpv_configuration', { text }))}
        >Save MPV configuration</button
      >
      <p class="muted">Changes apply when starting the next player.</p>
      {#each settings.plugins as name (name)}<div class="row">
          <span>{name}</span><button
            class="secondary"
            disabled={busy}
            onclick={() =>
              void act(() => invoke('mpv_plugin', { name, text: null }))}
            >Remove plugin</button
          >
        </div>{/each}
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void act(
            () => invoke('mpv_plugin', { name: pluginName, text: pluginText }),
            'Plugin saved',
          );
        }}
      >
        <label
          >Plugin filename<input
            bind:value={pluginName}
            required
            pattern="[A-Za-z0-9_-][A-Za-z0-9_.-]*\.lua"
            placeholder="my-player.lua"
          /></label
        ><label
          >Lua plugin<textarea
            bind:value={pluginText}
            required
            rows="6"
            spellcheck={false}></textarea></label
        ><button class="secondary" disabled={busy}>Save plugin</button>
      </form>
    </details>
    <p class="muted">
      Desktop {settings.product_version}
    </p>{/if}
</section>
