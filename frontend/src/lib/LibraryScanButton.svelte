<script lang="ts">
  import { RefreshCw } from '@lucide/svelte';
  import { showToast } from './providers/toasts';
  import { loadRoots, scanRoot } from './title/catalog';
  import Button from './ui/Button.svelte';

  let { domain }: { domain: 'Movies' | 'Shows' | 'Music' } = $props();
  let scanning = $state(false);
  async function scan() {
    scanning = true;
    try {
      const roots = (await loadRoots()).filter(
        (root) => root.kind === domain.toLowerCase(),
      );
      await Promise.all(roots.map((root) => scanRoot(root.id)));
      showToast({
        key: `scan:${domain}`,
        tone: roots.length ? 'success' : 'warning',
        title: domain,
        message: roots.length
          ? `Scanning ${roots.length} library ${roots.length === 1 ? 'folder' : 'folders'}.`
          : 'No accessible media folders have been detected yet.',
      });
    } catch (e) {
      showToast({
        key: `scan:${domain}`,
        tone: 'error',
        title: domain,
        message: String(e),
      });
    } finally {
      scanning = false;
    }
  }
</script>

<Button
  variant="secondary"
  size="icon"
  aria-label={`Scan ${domain.toLowerCase()} library`}
  title="Scan library"
  loading={scanning}
  onclick={() => void scan()}><RefreshCw size={16} /></Button
>
