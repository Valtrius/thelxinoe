<script lang="ts">
  import type { MpvOption } from '../../tools-api';
  import { parseMpvConfig, setMpvOption } from '../../mpv-config';
  import Switch from '../ui/Switch.svelte';
  import MpvOptionField from './MpvOptionField.svelte';
  import { controlClass } from './settingsUi';

  let {
    options,
    text = $bindable(),
    search = $bindable(''),
    all = $bindable(false),
    disabled = false,
    onError,
  }: {
    options: MpvOption[];
    text: string;
    search?: string;
    all?: boolean;
    disabled?: boolean;
    onError: (error: unknown) => void;
  } = $props();
  const common = new Set([
    'volume',
    'volume-max',
    'mute',
    'fullscreen',
    'ontop',
    'border',
    'keep-open',
    'save-position-on-quit',
    'hwdec',
    'vo',
    'video-sync',
    'interpolation',
    'scale',
    'cscale',
    'dscale',
    'deband',
    'sub-scale',
    'sub-font',
    'sub-font-size',
    'sub-color',
    'sub-border-size',
    'sub-pos',
    'alang',
    'slang',
    'audio-device',
    'audio-delay',
    'speed',
    'screenshot-format',
    'screenshot-dir',
    'cache',
    'demuxer-max-bytes',
  ]);
  const parsed = $derived(parseMpvConfig(text, options));
  const filtered = $derived(
    options.filter(
      (option) =>
        /^(Flag|Choice|Integer|Integer64|Float|Double|String|String list)$/.test(
          option.type,
        ) &&
        (search
          ? option.name.includes(search.toLowerCase())
          : all || common.has(option.name) || parsed.values.has(option.name)),
    ),
  );
  function updateOption(name: string, value: string | null) {
    try {
      text = setMpvOption(text, name, value, options);
    } catch (error) {
      onError(error);
    }
  }
</script>

<div class="flex min-h-40 flex-1 flex-col gap-2">
  <div class="flex flex-wrap items-center gap-3">
    <label class="min-w-40 flex-1 text-xs">
      <span class="sr-only">Find an option</span>
      <input
        class={controlClass}
        type="search"
        bind:value={search}
        placeholder="Search all available options…"
      />
    </label>
    <Switch size="sm" class="text-xs" bind:checked={all}
      >Show all supported options</Switch
    >
  </div>
  {#if parsed.complex}
    <p class="text-xs text-(--warning)">
      This file contains complex or unsupported syntax. Use the raw editor to
      preserve it.
    </p>
  {/if}
  <div class="max-h-96 min-h-32 flex-1 overflow-y-auto border border-(--line)">
    {#each filtered.slice(0, 100) as option (option.name)}
      <MpvOptionField
        {option}
        value={parsed.values.get(option.name)?.at(-1)?.value}
        {disabled}
        requiresRaw={parsed.complex || parsed.blocked.has(option.name)}
        onChange={(value) => updateOption(option.name, value)}
      />
    {/each}
  </div>
  {#if filtered.length > 100}<p class="text-xs text-(--muted)">
      Showing the first 100 matches. Refine your search.
    </p>{/if}
</div>
