<script lang="ts">
  import { attentionDescription, type AttentionItem } from '../attention';
  let { items, class: className = '' } = $props<{
    items: AttentionItem[];
    class?: string;
  }>();
  const severity = $derived(
    items.some((item: AttentionItem) => item.severity === 'error')
      ? 'error'
      : items.some((item: AttentionItem) => item.severity === 'warning')
        ? 'warning'
        : 'info',
  );
</script>

{#if items.length}
  <span
    data-attention-severity={severity}
    role="img"
    aria-label={attentionDescription(items)}
    title={attentionDescription(items)}
    class={[
      'inline-block size-1.75 shrink-0 rounded-full',
      severity === 'error'
        ? 'bg-danger'
        : severity === 'warning'
          ? 'bg-warning'
          : 'bg-accent',
      className,
    ]}
  ></span>
{/if}
