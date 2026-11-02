<script lang="ts">
  import { onMount } from 'svelte';
  import { mediaUrl } from './playback';
  import type { StyledRenderer } from './styled-subtitles';

  let {
    video,
    url,
    fonts,
    offset,
    size,
    failed,
  }: {
    video: HTMLVideoElement;
    /** The track in its own styled format. */
    url: string;
    fonts: string[];
    /** Seconds from the video's time to the track's. */
    offset: number;
    size: number;
    failed: () => void;
  } = $props();
  let canvas: HTMLCanvasElement;
  let renderer = $state<StyledRenderer>();
  let script = '';
  let scaled: (script: string, size: number) => string = (script) => script;
  let shownSize = 1;

  onMount(() => {
    let closed = false;
    let instance: StyledRenderer | undefined;
    void (async () => {
      const [response, styled] = await Promise.all([
        fetch(mediaUrl(url)),
        import('./styled-subtitles'),
      ]);
      if (!response.ok)
        throw new Error(`Subtitle request failed: ${response.status}`);
      script = await response.text();
      if (closed) return;
      scaled = styled.scaled;
      // The renderer's worker resolves addresses against its own script.
      const absolute = (path: string) =>
        new URL(mediaUrl(path), location.href).href;
      shownSize = size;
      instance = styled.render(
        video,
        canvas,
        scaled(script, shownSize),
        fonts.map(absolute),
        offset,
      );
      await instance.ready;
      if (!closed) renderer = instance;
    })().catch((error) => {
      console.warn('Styled subtitles are unavailable', error);
      if (!closed) failed();
    });
    return () => {
      closed = true;
      void instance?.destroy();
    };
  });
  $effect(() => {
    if (!renderer) return;
    renderer.timeOffset = offset;
    void renderer.resize();
  });
  $effect(() => {
    if (!renderer || size === shownSize) return;
    shownSize = size;
    void renderer.renderer.setTrack(scaled(script, size));
    void renderer.resize();
  });
</script>

<canvas
  bind:this={canvas}
  class="styled-subtitles pointer-events-none absolute"
  aria-hidden="true"
></canvas>
