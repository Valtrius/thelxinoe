<script module lang="ts">
  // A menu that closed on the press would remove the backdrop before the
  // click, which then lands on whatever is under it. Pass this as the menu's
  // `onInteractOutside` so the backdrop's click closes it instead.
  export function ignoreBackdrop(event: PointerEvent) {
    if (
      event.target instanceof Element &&
      event.target.closest('[data-menu-backdrop]')
    )
      event.preventDefault();
  }
</script>

<script lang="ts">
  import { twMerge } from 'tailwind-merge';
  let { close, class: className = '' } = $props<{
    close: () => void;
    class?: string;
  }>();
  // A tap opens a dropdown menu on its pointerup, so Chromium sends the tap's
  // click here. Only a press that started on the backdrop closes the menu.
  let pressed = false;
</script>

<button
  type="button"
  data-menu-backdrop
  tabindex="-1"
  aria-hidden="true"
  class={twMerge('fixed inset-0 z-40 cursor-default', className)}
  onpointerdown={() => (pressed = true)}
  onclick={() => {
    if (pressed) close();
  }}
></button>
