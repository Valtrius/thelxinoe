<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { twMerge } from 'tailwind-merge';
  import { onDestroy } from 'svelte';
  import { api, type User } from './api';
  import { cropRectangle } from './avatar-crop';
  import Panel from './ui/Panel.svelte';
  import Button from './ui/Button.svelte';

  import { Pencil } from '@lucide/svelte';

  let { user, changed } = $props<{
    user: User;
    changed: (id: string, avatar: string | null) => void;
  }>();
  let image = $state.raw<HTMLImageElement | null>(null);
  let zoom = $state(1),
    horizontal = $state(0.5),
    vertical = $state(0.5);
  let resolution = $state(256),
    previewSize = $state(256);
  let error = $state(''),
    busy = $state(false),
    saved = $state(false);
  let sourceUrl = '',
    generation = 0;
  let drag:
    | { pointer: number; x: number; y: number; cropX: number; cropY: number }
    | undefined;
  const crop = $derived(
    image
      ? cropRectangle(
          image.naturalWidth,
          image.naturalHeight,
          zoom,
          horizontal,
          vertical,
        )
      : { x: 0, y: 0, size: 1 },
  );
  const scale = $derived(previewSize / crop.size);

  function clear() {
    generation++;
    URL.revokeObjectURL(sourceUrl);
    sourceUrl = '';
    image = null;
    drag = undefined;
  }
  async function choose(file: File | undefined) {
    if (!file) return;
    clear();
    error = '';
    saved = false;
    if (
      !['image/jpeg', 'image/png', 'image/webp', 'image/gif'].includes(
        file.type,
      ) ||
      file.size > 15 * 1024 * 1024
    ) {
      error = 'Choose a JPEG, PNG, WebP or GIF picture smaller than 15 MB.';
      return;
    }
    const revision = generation;
    sourceUrl = URL.createObjectURL(file);
    const loaded = new Image();
    loaded.src = sourceUrl;
    try {
      await loaded.decode();
      if (generation !== revision) return;
      if (
        !loaded.naturalWidth ||
        !loaded.naturalHeight ||
        loaded.naturalWidth * loaded.naturalHeight > 64_000_000
      )
        throw new Error('Choose a picture smaller than 64 megapixels.');
      zoom = 1;
      horizontal = vertical = 0.5;
      image = loaded;
    } catch (caught) {
      if (generation === revision) {
        clear();
        error = String(caught);
      }
    }
  }
  async function save(remove = false) {
    const userId = user.id;
    error = '';
    saved = false;
    busy = true;
    try {
      let picture: string | null = null;
      if (!remove) {
        if (!image) return;
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = resolution;
        const context = canvas.getContext('2d');
        if (!context)
          throw new Error('Your browser could not prepare the picture.');
        context.fillStyle = '#ffffff';
        context.fillRect(0, 0, resolution, resolution);
        context.drawImage(
          image,
          crop.x,
          crop.y,
          crop.size,
          crop.size,
          0,
          0,
          resolution,
          resolution,
        );
        picture = canvas.toDataURL('image/jpeg', 0.9);
      }
      const result = await api<{ avatar: string | null }>('/me/avatar', 'PUT', {
        image: picture,
      });
      changed(userId, result.avatar);
      clear();
      saved = true;
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
  onDestroy(clear);
</script>

<Panel>
  <h2>Profile picture</h2>
  <div class="flex flex-wrap items-center gap-4">
    <FormField
      class="group/avatar relative mb-0 grid size-16 shrink-0 cursor-pointer place-items-center overflow-hidden rounded-full border border-line bg-surface-soft text-2xl text-accent"
    >
      {#if user.avatar}<img
          src={user.avatar}
          alt={user.username}
          class="size-full object-cover"
        />
      {:else}{user.username[0].toUpperCase()}{/if}
      <span
        class="pointer-events-none absolute inset-0 grid place-items-center bg-black/55 text-white opacity-0 transition-opacity group-hover/avatar:opacity-100 group-focus-within/avatar:opacity-100"
        aria-hidden="true"><Pencil size={20} /></span
      >
      <input
        class={twMerge(formControlClass, 'sr-only')}
        type="file"
        aria-label="Change profile picture"
        accept="image/jpeg,image/png,image/webp,image/gif"
        disabled={busy}
        onchange={(event) => {
          void choose(event.currentTarget.files?.[0]);
          event.currentTarget.value = '';
        }}
      />
    </FormField>
    {#if user.avatar}<Button
        size="form"
        variant="secondary"
        disabled={busy}
        onclick={() => void save(true)}>Remove picture</Button
      >{/if}
  </div>
  {#if image}
    <div class="mt-5 grid max-w-120 gap-4">
      <p class="mb-0 text-muted">
        Drag the picture to reframe it, or use the arrow keys. Adjust the zoom
        and saved size below.
      </p>
      <button
        type="button"
        aria-label="Reframe profile picture"
        disabled={busy}
        class="relative aspect-square w-64 max-w-full touch-none cursor-grab overflow-hidden rounded-full border border-line bg-surface-soft active:cursor-grabbing"
        bind:clientWidth={previewSize}
        onpointerdown={(event) => {
          if (event.button !== 0 || !event.isPrimary) return;
          event.preventDefault();
          drag = {
            pointer: event.pointerId,
            x: event.clientX,
            y: event.clientY,
            cropX: crop.x,
            cropY: crop.y,
          };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onpointermove={(event) => {
          if (!drag || drag.pointer !== event.pointerId || !image) return;
          const width = image.naturalWidth - crop.size,
            height = image.naturalHeight - crop.size;
          horizontal =
            width > 0
              ? Math.max(
                  0,
                  Math.min(
                    1,
                    (drag.cropX - (event.clientX - drag.x) / scale) / width,
                  ),
                )
              : 0.5;
          vertical =
            height > 0
              ? Math.max(
                  0,
                  Math.min(
                    1,
                    (drag.cropY - (event.clientY - drag.y) / scale) / height,
                  ),
                )
              : 0.5;
        }}
        onpointerup={(event) => {
          drag = undefined;
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onlostpointercapture={() => (drag = undefined)}
        onpointercancel={() => (drag = undefined)}
        onkeydown={(event) => {
          if (event.key === 'ArrowLeft')
            horizontal = Math.max(0, horizontal - 0.05);
          else if (event.key === 'ArrowRight')
            horizontal = Math.min(1, horizontal + 0.05);
          else if (event.key === 'ArrowUp')
            vertical = Math.max(0, vertical - 0.05);
          else if (event.key === 'ArrowDown')
            vertical = Math.min(1, vertical + 0.05);
          else return;
          event.preventDefault();
        }}
      >
        <img
          src={image.src}
          alt=""
          draggable="false"
          class="pointer-events-none absolute top-0 left-0 max-w-none select-none"
          style:width={`${image.naturalWidth * scale}px`}
          style:height={`${image.naturalHeight * scale}px`}
          style:transform={`translate(${-crop.x * scale}px, ${-crop.y * scale}px)`}
        />
      </button>
      <FormField class="mb-0"
        >Picture zoom<input
          class={formControlClass}
          type="range"
          min="1"
          max="4"
          step="0.01"
          bind:value={zoom}
          disabled={busy}
        /></FormField
      >
      <FormField class="mb-0"
        >Saved picture size<select
          class={formControlClass}
          bind:value={resolution}
          disabled={busy}
        >
          <option value={128}>128 × 128 pixels</option><option value={256}
            >256 × 256 pixels</option
          ><option value={512}>512 × 512 pixels</option>
        </select></FormField
      >
      <div class="flex gap-2">
        <Button size="form" disabled={busy} onclick={() => void save()}
          >{busy ? 'Saving picture…' : 'Save picture'}</Button
        >
        <Button size="form" variant="secondary" disabled={busy} onclick={clear}
          >Cancel</Button
        >
      </div>
    </div>
  {/if}
  {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
  {#if saved}<p role="status">Profile picture saved.</p>{/if}
</Panel>
