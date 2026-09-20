<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { FolderOpen, Package, Search } from '@lucide/svelte';
  import { toolsApi, type MpvPreferences } from '../../tools-api';
  import { toolsState } from '../../tools-state';
  import { displayToolPath } from '../../tools-presentation';
  import Button from '../ui/Button.svelte';

  let {
    preferences,
    directory,
    onError,
  }: {
    preferences: MpvPreferences;
    directory: string;
    onError: (error: unknown) => void;
  } = $props();
  let choosingDirectory = $state(false);

  async function chooseSource(
    source: MpvPreferences['source'],
    chooseDirectory = false,
  ) {
    if (choosingDirectory) return;
    try {
      let selectedDirectory = preferences.directory;
      if (source === 'directory' && (chooseDirectory || !selectedDirectory)) {
        choosingDirectory = true;
        const result = await open({
          directory: true,
          multiple: false,
          title: 'Select your MPV configuration directory',
        });
        if (typeof result !== 'string') return;
        selectedDirectory = result;
      }
      await toolsState.mpvPreferences({ source, directory: selectedDirectory });
    } catch (error) {
      onError(error);
    } finally {
      choosingDirectory = false;
    }
  }

  async function openDirectory() {
    try {
      if (await toolsState.settle('configuration'))
        await toolsApi.openConfigurationDirectory();
    } catch (error) {
      onError(error);
    }
  }
</script>

<div class="grid gap-2">
  <fieldset disabled={choosingDirectory} class="grid gap-2">
    <div class="flex flex-wrap gap-2">
      {#each [{ source: 'native' as const, name: "Use MPV's normal configuration", icon: Search }, { source: 'directory' as const, name: 'Use a configuration folder', icon: FolderOpen }, { source: 'managed' as const, name: 'Manage a copy in Thelxinoe', icon: Package }] as choice (choice.source)}
        <button
          type="button"
          aria-pressed={preferences.source === choice.source}
          onclick={() => {
            if (preferences.source !== choice.source)
              void chooseSource(choice.source);
          }}
          class={`flex cursor-pointer items-center gap-2 border px-3 py-2 text-left disabled:cursor-default disabled:opacity-45 ${preferences.source === choice.source ? 'border-(--line-strong) bg-(--accent-soft)' : 'border-(--line) bg-(--surface-soft)'}`}
        >
          <span class="flex items-center gap-2 text-xs font-semibold"
            ><choice.icon class="size-3.5" />{choice.name}</span
          >
        </button>
      {/each}
    </div>
  </fieldset>
  {#if preferences.source === 'directory'}
    <div class="flex flex-wrap items-center gap-2">
      <p class="min-w-0 flex-1 font-mono text-xs wrap-anywhere text-(--muted)">
        {displayToolPath(preferences.directory)}
      </p>
      <Button
        variant="secondary"
        size="sm"
        disabled={choosingDirectory}
        onclick={() => chooseSource('directory', true)}>Choose folder...</Button
      >
    </div>
  {/if}
  {#if preferences.source !== 'native'}
    <div class="flex flex-wrap items-center gap-2">
      {#if preferences.source === 'managed'}
        <p
          class="min-w-0 flex-1 font-mono text-xs wrap-anywhere text-(--muted)"
        >
          {displayToolPath(directory + '/config/mpv')}
        </p>
      {/if}
      <Button variant="ghost" size="sm" onclick={openDirectory}
        >Open folder</Button
      >
    </div>
  {/if}
</div>
