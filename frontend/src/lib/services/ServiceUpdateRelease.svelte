<script lang="ts">
  import { ExternalLink } from '@lucide/svelte';

  export type ServiceRelease = {
    image: string;
    version: string | null;
    build_version: string | null;
    release_notes_url: string | null;
  };

  let {
    image,
    installedVersion,
    release,
  }: {
    image: string;
    installedVersion?: string;
    release?: ServiceRelease | null;
  } = $props();
  const metadata = $derived(release?.image === image ? release : null);
  const containerUpdate = $derived(
    !!metadata?.version &&
      metadata.version === installedVersion?.replace(/^v/, ''),
  );
</script>

<div class="candidate my-3 grid gap-2 text-xs">
  {#if metadata?.version}
    <p class="m-0">
      {containerUpdate
        ? 'Container update available'
        : `Available version: ${metadata.version}`}
    </p>
    {#if containerUpdate}<p class="m-0 text-muted">
        Application version: {metadata.version}
      </p>{/if}
    {#if metadata.release_notes_url && !containerUpdate}
      <a
        href={metadata.release_notes_url}
        target="_blank"
        rel="noopener noreferrer"
        class="inline-flex w-fit items-center gap-1.5 text-accent hover:underline"
      >
        Release notes <ExternalLink size={12} aria-hidden="true" />
      </a>
    {/if}
    <details class="text-muted">
      <summary class="w-fit cursor-pointer hover:text-foreground"
        >Image details</summary
      >
      <dl class="mt-2 grid gap-2 text-[11px]">
        {#if metadata.build_version}<div>
            <dt>Build</dt>
            <dd class="wrap-anywhere">{metadata.build_version}</dd>
          </div>{/if}
        <div>
          <dt>Image</dt>
          <dd class="wrap-anywhere"><code>{image}</code></dd>
        </div>
      </dl>
    </details>
  {:else}
    <p class="m-0 wrap-anywhere text-[11px] leading-[1.6] text-muted">
      Available image: <code>{image}</code>
    </p>
  {/if}
</div>
