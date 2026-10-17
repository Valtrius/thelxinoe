<script lang="ts">
  import { onMount } from 'svelte';

  // "On this page" row for long settings pages. Sections are looked up by id
  // on every scroll because most of them load lazily.
  let { items }: { items: { id: string; label: string }[] } = $props();
  let active = $state('');
  let nav = $state<HTMLElement>();

  // The cards scroll inside `.settings-scroll` on wide layouts and with the
  // page on compact ones.
  function scroller(): HTMLElement | Window {
    const inner = document.querySelector<HTMLElement>('.settings-scroll');
    if (inner && getComputedStyle(inner).overflowY !== 'visible') return inner;
    return nav?.closest<HTMLElement>('.workspace-scroll') ?? window;
  }
  function update() {
    const root = scroller();
    const top = root instanceof Window ? 0 : root.getBoundingClientRect().top;
    let current = items[0]?.id ?? '';
    for (const item of items) {
      const element = document.getElementById(item.id);
      if (element && element.getBoundingClientRect().top - top <= 80)
        current = item.id;
    }
    active = current;
  }
  function jump(id: string) {
    const element = document.getElementById(id);
    if (!element) return;
    active = id;
    element.scrollIntoView({ block: 'start', behavior: 'smooth' });
  }
  onMount(() => {
    let root = scroller();
    const rebind = () => {
      root.removeEventListener('scroll', update);
      root = scroller();
      root.addEventListener('scroll', update, { passive: true });
      update();
    };
    root.addEventListener('scroll', update, { passive: true });
    window.addEventListener('resize', rebind);
    update();
    return () => {
      root.removeEventListener('scroll', update);
      window.removeEventListener('resize', rebind);
    };
  });
</script>

<nav
  bind:this={nav}
  aria-label="On this page"
  class="mb-5 flex flex-wrap gap-1.5"
>
  {#each items as item (item.id)}
    <button
      type="button"
      aria-current={active === item.id ? 'location' : undefined}
      class="border border-line bg-surface-strong px-2.5 py-1.25 text-[11px] font-semibold text-muted transition-colors hover:text-foreground aria-[current=location]:border-line-strong aria-[current=location]:bg-accent-soft aria-[current=location]:text-accent"
      onclick={() => jump(item.id)}>{item.label}</button
    >
  {/each}
</nav>
