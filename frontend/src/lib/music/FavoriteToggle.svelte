<script lang="ts">
  import { Heart } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import { showToast } from '../providers/toasts';
  import { loadFlags, setFlag } from '../title/catalog';
  let {
    id,
    variant = 'secondary',
    class: className = '',
  }: {
    id: string;
    variant?: 'secondary' | 'player';
    class?: string;
  } = $props();
  let favorite = $state<boolean | null>(null),
    busy = $state(false);
  // Only another id reads the flags again, not another object holding this one.
  const media = $derived(id);
  $effect(() => {
    const reading = media;
    favorite = null;
    loadFlags(reading).then(
      (flags) => {
        if (media === reading) favorite = flags.favorite;
      },
      () => {},
    );
  });
  async function toggle() {
    if (favorite === null) return;
    const changing = media;
    busy = true;
    try {
      const flags = await setFlag(changing, 'favorite', !favorite);
      if (media === changing) favorite = flags.favorite;
    } catch (e) {
      showToast({
        key: `favorite:${changing}`,
        tone: 'error',
        title: 'Favorites',
        message: String(e),
      });
    } finally {
      busy = false;
    }
  }
</script>

<Button
  {variant}
  size="icon"
  class={className}
  disabled={favorite === null || busy}
  aria-pressed={favorite ?? false}
  aria-label={favorite ? 'Remove from Favorites' : 'Add to Favorites'}
  title={favorite ? 'Remove from Favorites' : 'Add to Favorites'}
  onclick={() => void toggle()}
  ><Heart
    size={16}
    fill={favorite ? 'currentColor' : 'none'}
    class={favorite ? 'text-accent' : ''}
  /></Button
>
