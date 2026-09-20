<script lang="ts">
  let {
    platform,
    class: className = '',
  }: {
    platform: 'youtube' | 'twitch' | 'kick';
    class?: string;
  } = $props();
</script>

{#if platform === 'youtube'}
  <svg class={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path
      d="M21.4 6.2c-.2-.9-.9-1.6-1.8-1.8C18 4 12 4 12 4s-6 0-7.6.4c-.9.2-1.6.9-1.8 1.8C2.2 7.8 2.2 12 2.2 12s0 4.2.4 5.8c.2.9.9 1.6 1.8 1.8C6 20 12 20 12 20s6 0 7.6-.4c.9-.2 1.6-.9 1.8-1.8.4-1.6.4-5.8.4-5.8s0-4.2-.4-5.8Z"
      fill="currentColor"
    />
    <path d="m10 15.4 5.2-3.4L10 8.6v6.8Z" fill="var(--surface)" />
  </svg>
{:else if platform === 'twitch'}
  <svg class={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path
      d="M4 3h17v11.4L16.4 19H13l-2.2 2.2H8.6V19H4V3Zm2.2 2.2v11.6h4.6V19l2.2-2.2h3l2.8-2.8V5.2H6.2Z"
      fill="currentColor"
    />
    <path d="M10 8h2.2v5H10V8Zm5 0h2.2v5H15V8Z" fill="currentColor" />
  </svg>
{:else}
  <svg class={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path
      d="M4 3h5v6.2L14.1 3H20l-6.8 8.1L20.5 21h-6.1L9 13.7V21H4V3Z"
      fill="currentColor"
    />
  </svg>
{/if}
