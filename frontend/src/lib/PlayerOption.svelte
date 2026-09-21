<script lang="ts">
  import type { Snippet } from 'svelte';
  import { DropdownMenu } from 'bits-ui';
  import { Check } from '@lucide/svelte';

  let {
    label,
    icon,
    value,
    options,
    disabled,
    open,
    setOpen,
    select,
  }: {
    label: string;
    icon: Snippet;
    value: string;
    options: { value: string; label: string }[];
    disabled: boolean;
    open: boolean;
    setOpen: (open: boolean) => void;
    select: (value: string) => void;
  } = $props();
  const current = $derived(
    options.find((option) => option.value === value)?.label,
  );
</script>

<div class="player-option">
  <DropdownMenu.Root {open} onOpenChange={setOpen}>
    <DropdownMenu.Trigger
      class="option-button"
      aria-label={label}
      title={`${label}: ${current ?? 'Default'}`}
      {disabled}
    >
      {@render icon()}
    </DropdownMenu.Trigger>
    <DropdownMenu.ContentStatic
      class="option-menu"
      aria-label={label}
      preventScroll={false}
    >
      <DropdownMenu.RadioGroup {value} onValueChange={select}>
        {#each options as option (option.value)}
          <DropdownMenu.RadioItem class="option-item" value={option.value}>
            <span class="option-check"
              >{#if option.value === value}<Check size={14} />{/if}</span
            >
            <span>{option.label}</span>
          </DropdownMenu.RadioItem>
        {/each}
      </DropdownMenu.RadioGroup>
    </DropdownMenu.ContentStatic>
  </DropdownMenu.Root>
</div>

<style>
  .player-option {
    position: relative;
  }
  .player-option :global(.option-button) {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    transition: background 150ms;
  }
  .player-option :global(.option-button:hover),
  .player-option :global(.option-button[data-state='open']) {
    background: #ffffff24;
  }
  .player-option :global(.option-button:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .player-option :global(.option-menu) {
    position: absolute;
    right: 0;
    bottom: calc(100% + 12px);
    z-index: 4;
    width: max-content;
    min-width: 140px;
    max-width: min(280px, 60cqw);
    max-height: max(60px, calc(var(--player-height) - 124px));
    overflow-y: auto;
    padding: 5px;
    border: 1px solid #ffffff24;
    background: #15191bf5;
    box-shadow: 0 8px 28px #0005;
    color: #f5f6f7;
    scrollbar-width: thin;
  }
  .player-option :global(.option-item) {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
    padding: 6px 10px;
    font-size: 12px;
    line-height: 1.4;
    cursor: pointer;
    overflow-wrap: anywhere;
    outline: none;
  }
  .player-option :global(.option-item[data-highlighted]) {
    background: #ffffff24;
  }
  .option-check {
    width: 14px;
    flex-shrink: 0;
    color: var(--accent);
  }
</style>
