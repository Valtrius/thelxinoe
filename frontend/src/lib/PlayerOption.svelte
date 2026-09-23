<script lang="ts">
  import type { Snippet } from 'svelte';
  import { DropdownMenu } from 'bits-ui';
  import { Check } from '@lucide/svelte';
  import Button from './ui/Button.svelte';

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

<div class="player-option relative">
  <DropdownMenu.Root {open} onOpenChange={setOpen}>
    <DropdownMenu.Trigger
      aria-label={label}
      title={`${label}: ${current ?? 'Default'}`}
      {disabled}
    >
      {#snippet child({ props })}
        <Button {...props} variant="player" size="icon" class="option-button">
          {@render icon()}
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.ContentStatic
      class="option-menu absolute right-0 bottom-[calc(100%+12px)] z-[4] max-h-[max(60px,calc(var(--player-height)-124px))] w-max min-w-[140px] max-w-[min(280px,60cqw)] overflow-y-auto border border-[#ffffff24] bg-[#15191bf5] p-[5px] text-[#f5f6f7] shadow-[0_8px_28px_#0005] [scrollbar-width:thin]"
      aria-label={label}
      preventScroll={false}
    >
      <DropdownMenu.RadioGroup {value} onValueChange={select}>
        {#each options as option (option.value)}
          <DropdownMenu.RadioItem
            class="option-item flex min-h-8 cursor-pointer items-center gap-2 px-2.5 py-1.5 text-xs leading-[1.4] wrap-anywhere outline-none data-[highlighted]:bg-[#ffffff24]"
            value={option.value}
          >
            <span class="option-check w-3.5 shrink-0 text-accent"
              >{#if option.value === value}<Check size={14} />{/if}</span
            >
            <span>{option.label}</span>
          </DropdownMenu.RadioItem>
        {/each}
      </DropdownMenu.RadioGroup>
    </DropdownMenu.ContentStatic>
  </DropdownMenu.Root>
</div>
