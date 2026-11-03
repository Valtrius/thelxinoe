<script lang="ts">
  import { Heart } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import { showToast } from '../providers/toasts';
  import { loadFlags, setFlag } from '../title/catalog';
  let {
    id,
    variant = 'secondary',
    class: className = '',
  } = $props<{
    id: string;
    variant?: 'secondary' | 'player';
    class?: string;
  }>();
  let favorite = $state<boolean | null>(null),
    busy = $state(false);
  $effect(() => {
    const media = id;
    favorite = null;
    loadFlags(media).then(
      (flags) => {
        if (id === media) favorite = flags.favorite;
      },
      () => {},
    );
  });
  async function toggle() {
    if (favorite === null) return;
    const media = id;
    busy = true;
    try {
      const flags = await setFlag(media, 'favorite', !favorite);
      if (id === media) favorite = flags.favorite;
    } catch (e) {
      showToast({
        key: `favorite:${media}`,
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
