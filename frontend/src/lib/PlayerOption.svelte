<script lang="ts">
  import type { Snippet } from 'svelte';
  import { DropdownMenu } from 'bits-ui';
  import { Check, Minus, Plus } from '@lucide/svelte';
  import Button from './ui/Button.svelte';
  import MenuBackdrop, { ignoreBackdrop } from './ui/MenuBackdrop.svelte';
  import { eyebrowTextClass } from './ui/styles';

  let {
    label,
    icon,
    value,
    options,
    steppers = [],
    disabled,
    open,
    setOpen,
    select,
  }: {
    label: string;
    icon: Snippet;
    value: string;
    options: { value: string; label: string }[];
    /** Settings below the options that step a value down or up. */
    steppers?: {
      label: string;
      value: string;
      less: string;
      more: string;
      step: (direction: -1 | 1) => void;
    }[];
    disabled: boolean;
    open: boolean;
    setOpen: (open: boolean) => void;
    select: (value: string) => void;
  } = $props();
  const current = $derived(
    options.find((option) => option.value === value)?.label,
  );
  const stepClass =
    'option-step grid size-7 shrink-0 cursor-pointer place-items-center outline-none data-[highlighted]:bg-accent-soft data-[highlighted]:text-accent';
</script>

<div class="player-option relative">
  <DropdownMenu.Root {open} onOpenChange={setOpen}>
    <DropdownMenu.Trigger
      aria-label={label}
      title={`${label}: ${current ?? 'Default'}`}
      {disabled}
    >
      {#snippet child({ props })}
        <!-- Above every option menu's backdrop, so one press moves between menus. -->
        <Button
          {...props}
          variant="player"
          size="icon"
          class="option-button relative z-[4]"
        >
          {@render icon()}
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>
    {#if open}<MenuBackdrop class="z-[3]" close={() => setOpen(false)} />{/if}
    <DropdownMenu.ContentStatic
      class="option-menu absolute right-0 bottom-[calc(100%+12px)] z-[4] max-h-[max(60px,calc(var(--player-height)-124px))] w-max min-w-[140px] max-w-[min(280px,60cqw)] overflow-y-auto border border-line bg-surface-strong p-1 text-foreground shadow-card [scrollbar-width:thin]"
      aria-label={label}
      preventScroll={false}
      onInteractOutside={ignoreBackdrop}
    >
      <div
        class={['px-2.5 pt-1.75 pb-1.25', eyebrowTextClass]}
        aria-hidden="true"
      >
        {label}
      </div>
      <DropdownMenu.RadioGroup {value} onValueChange={select}>
        {#each options as option (option.value)}
          <DropdownMenu.RadioItem
            class="option-item flex min-h-8 cursor-pointer items-center gap-2 px-2.5 py-1.5 text-xs leading-[1.4] wrap-anywhere outline-none data-[highlighted]:bg-accent-soft data-[state=checked]:text-accent"
            value={option.value}
          >
            <span class="option-check w-3.5 shrink-0 text-accent"
              >{#if option.value === value}<Check size={14} />{/if}</span
            >
            <span>{option.label}</span>
          </DropdownMenu.RadioItem>
        {/each}
      </DropdownMenu.RadioGroup>
      {#if steppers.length}
        <DropdownMenu.Separator class="mx-1 my-1 h-px bg-line" />
        {#each steppers as stepper (stepper.label)}
          <DropdownMenu.Group
            class="option-stepper flex min-h-8 items-center gap-1 pl-2.5 text-xs"
            aria-label={stepper.label}
          >
            <span class="min-w-0 flex-1 text-muted">{stepper.label}</span>
            <DropdownMenu.Item
              class={stepClass}
              aria-label={stepper.less}
              title={stepper.less}
              closeOnSelect={false}
              onSelect={() => stepper.step(-1)}
            >
              <Minus size={14} />
            </DropdownMenu.Item>
            <span class="w-12 text-center tabular-nums">{stepper.value}</span>
            <DropdownMenu.Item
              class={stepClass}
              aria-label={stepper.more}
              title={stepper.more}
              closeOnSelect={false}
              onSelect={() => stepper.step(1)}
            >
              <Plus size={14} />
            </DropdownMenu.Item>
          </DropdownMenu.Group>
        {/each}
      {/if}
    </DropdownMenu.ContentStatic>
  </DropdownMenu.Root>
</div>
