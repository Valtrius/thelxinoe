<script lang="ts">
  import { onDestroy } from 'svelte';
  import { Popover } from 'bits-ui';
  import { Check, ChevronDown, LoaderCircle, Plus } from '@lucide/svelte';
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import { formControlClass } from '../ui/styles';
  import type { MediaKind } from './types';

  export type ProfileRef =
    | { source: 'guide'; service_id: string; trash_id: string }
    | { source: 'custom'; service_id: string; profile_id: number };
  type Choice = { name: string; default: boolean; profile: ProfileRef };
  let { mediaKind, id, disabled, busy, inactiveLabel, request } = $props<{
    mediaKind: MediaKind;
    id: number;
    disabled: boolean;
    busy: boolean;
    inactiveLabel: string;
    request: (profile?: ProfileRef) => void;
  }>();
  let items = $state<Choice[]>([]),
    locked = $state(false),
    loading = $state(false),
    error = $state(''),
    open = $state(false),
    query = $state('');
  let generation = 0;
  const defaultProfile = $derived(items.find((item) => item.default));
  const others = $derived(
    items.filter(
      (item) =>
        !item.default && item.name.toLowerCase().includes(query.toLowerCase()),
    ),
  );
  async function load() {
    const version = ++generation;
    loading = true;
    error = '';
    try {
      const result = await api<{ items: Choice[]; locked: boolean }>(
        `/seerr/profiles/${mediaKind}?media_id=${id}`,
      );
      if (version !== generation) return;
      items = result.items;
      locked = result.locked;
      if (locked) open = false;
    } catch (caught) {
      if (version === generation) {
        items = [];
        error = String(caught);
      }
    } finally {
      if (version === generation) loading = false;
    }
  }
  $effect(() => {
    void mediaKind;
    void id;
    open = false;
    locked = false;
    query = '';
    void load();
  });
  onDestroy(() => generation++);
  function choose(profile: ProfileRef) {
    open = false;
    request(profile);
  }
  function navigate(event: KeyboardEvent & { currentTarget: HTMLElement }) {
    const target = event.target as HTMLElement;
    const editing = target instanceof HTMLInputElement;
    if (
      !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key) ||
      (editing && ['Home', 'End'].includes(event.key))
    )
      return;
    const choices = Array.from(
      event.currentTarget.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]',
      ),
    );
    if (!choices.length) return;
    event.preventDefault();
    const index = choices.indexOf(target as HTMLButtonElement);
    const next =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? choices.length - 1
          : event.key === 'ArrowDown'
            ? (index + 1) % choices.length
            : index <= 0
              ? choices.length - 1
              : index - 1;
    choices[next].focus();
  }
</script>

<div class="mt-4 flex w-full">
  <Button
    class="min-w-0 flex-1 justify-center"
    size="form"
    {disabled}
    loading={busy}
    onclick={() => request()}
  >
    {#if disabled && !busy}<Check size={15} />{inactiveLabel}{:else}<Plus
        size={15}
      />Request{/if}
  </Button>
  {#if !locked && !disabled}
    <Popover.Root
      bind:open
      onOpenChange={(value) => {
        if (value) {
          query = '';
          void load();
        }
      }}
    >
      <Popover.Trigger aria-label="Choose request profile" disabled={busy}>
        {#snippet child({ props })}<Button
            {...props}
            size="form"
            class="border-l-0 px-2"><ChevronDown size={15} /></Button
          >{/snippet}
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          align="end"
          sideOffset={4}
          preventScroll={false}
          onkeydown={navigate}
          class="z-50 max-h-[min(28rem,80vh)] w-[min(22rem,calc(100vw-2rem))] overflow-y-auto border border-line-strong bg-surface-strong text-foreground"
        >
          {#if loading}<div
              role="status"
              class="flex items-center gap-2 px-3 py-4 text-xs text-muted"
            >
              <LoaderCircle
                size={14}
                class="animate-spin motion-reduce:animate-none"
              />Loading profiles…
            </div>
          {:else if error}<div class="px-3 py-3 text-xs">
              <p role="alert">{error}</p>
              <Button
                variant="ghost"
                size="sm"
                class="mt-2"
                onclick={() => void load()}>Retry</Button
              >
            </div>
          {:else}
            {#if defaultProfile}<div
                role="menu"
                aria-label="Default request profile"
              >
                <button
                  role="menuitem"
                  class="flex w-full items-center justify-between gap-3 bg-accent-soft px-3 py-3 text-left text-xs text-foreground hover:bg-accent/16"
                  onclick={() => choose(defaultProfile!.profile)}
                  ><span class="min-w-0 wrap-anywhere"
                    >{defaultProfile.name}</span
                  ><span class="shrink-0 text-[10px]">Default</span></button
                >
              </div>{/if}
            <div class="border-y border-line p-2">
              <input
                type="search"
                aria-label="Search request profiles"
                placeholder="Search profiles"
                bind:value={query}
                class={formControlClass}
              />
            </div>
            <div role="menu" aria-label="Other request profiles">
              {#each ['custom', 'guide'] as source (source)}
                {@const choices = others.filter(
                  (item) => item.profile.source === source,
                )}
                {#if choices.length}<p
                    class="px-3 pt-3 pb-1 text-[10px] font-semibold text-muted"
                  >
                    {source === 'guide' ? 'TRaSH Guides' : 'Custom profiles'}
                  </p>{/if}
                {#each choices as choice (JSON.stringify(choice.profile))}<button
                    role="menuitem"
                    class="block w-full px-3 py-2.5 text-left text-xs wrap-anywhere hover:bg-accent-soft focus-visible:bg-accent-soft"
                    onclick={() => choose(choice.profile)}>{choice.name}</button
                  >{/each}
              {/each}
              {#if !others.length}<p class="px-3 py-4 text-xs text-muted">
                  {query ? 'No matching profiles.' : 'No other profiles.'}
                </p>{/if}
            </div>
          {/if}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  {/if}
</div>
