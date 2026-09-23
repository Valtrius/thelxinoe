<script lang="ts">
  import { Search, Plus, X, ArrowLeft } from '@lucide/svelte';
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import FormField from '../ui/FormField.svelte';
  import Notice from '../ui/Notice.svelte';
  import Switch from '../ui/Switch.svelte';
  import { formControlClass } from '../ui/styles';
  let { serviceId, added } = $props<{
    serviceId: string;
    added: () => Promise<void>;
  }>();
  type Field = {
    name: string;
    label: string;
    type: string;
    value?: unknown;
    helpText?: string;
    advanced?: boolean;
    hidden?: string;
    privacy?: string;
    selectOptions?: { value: number | string; name: string }[];
    selectOptionsProviderAction?: string;
  };
  type Definition = {
    name: string;
    implementation: string;
    protocol: string;
    privacy: string;
    fields: Field[];
    infoLink?: string;
  };
  let expanded = $state(false),
    loading = $state(false),
    busy = $state(false),
    error = $state(''),
    message = $state(''),
    query = $state(''),
    protocol = $state('all'),
    definitions = $state<Definition[]>([]),
    profiles = $state<{ id: number; name: string }[]>([]),
    selected = $state<Definition | null>(null),
    values = $state<Record<string, unknown>>({}),
    name = $state(''),
    profile = $state(0),
    advanced = $state(false);
  let fieldBusy = $state<string | null>(null);
  let captchaImages = $state<Record<string, string>>({});
  const filtered = $derived(
    definitions.filter(
      (d) =>
        (protocol === 'all' || d.protocol === protocol) &&
        d.name.toLowerCase().includes(query.toLowerCase()),
    ),
  );
  const clean = (text?: string) => (text ?? '').replace(/<[^>]*>/g, '');
  async function start() {
    expanded = true;
    loading = true;
    error = '';
    try {
      const result = await api<{
        items: Definition[];
        profiles: { id: number; name: string }[];
      }>(`/admin/support/${serviceId}/indexers/schema`);
      definitions = result.items;
      profiles = result.profiles;
      profile = profiles[0]?.id ?? 0;
    } catch (caught) {
      error = String(caught);
    } finally {
      loading = false;
    }
  }
  function choose(definition: Definition) {
    selected = definition;
    name = definition.name;
    values = Object.fromEntries(
      definition.fields.map((f) => [
        f.name,
        f.value ??
          (f.type === 'checkbox'
            ? false
            : f.type === 'number'
              ? 0
              : f.type === 'tagSelect'
                ? []
                : ''),
      ]),
    );
    error = '';
    message = '';
    advanced = false;
    captchaImages = {};
    for (const field of definition.fields.filter(
      (field) => field.selectOptionsProviderAction === 'getUrls',
    ))
      void loadField(field);
  }
  const draft = () => ({
    implementation: selected!.implementation,
    definition: selected!.name,
    name,
    app_profile_id: profile,
    fields: values,
  });
  async function loadField(field: Field) {
    const current = selected;
    fieldBusy = field.name;
    error = '';
    try {
      const result = await api<{
        options?: { value: string; name: string }[];
        contentType?: string;
        imageData?: string;
      }>(`/admin/support/${serviceId}/indexers/fields`, 'POST', {
        draft: draft(),
        field: field.name,
      });
      if (selected !== current) return;
      if (result.options) {
        field.selectOptions = result.options;
        if (!values[field.name])
          values[field.name] = result.options[0]?.value ?? '';
      }
      if (result.contentType && result.imageData)
        captchaImages[field.name] =
          `data:${result.contentType};base64,${result.imageData}`;
    } catch (caught) {
      if (selected === current) error = String(caught);
    } finally {
      fieldBusy = null;
    }
  }
  async function submit(test: boolean) {
    if (!selected) return;
    busy = true;
    error = '';
    message = '';
    try {
      await api(
        `/admin/support/${serviceId}/indexers${test ? '/test' : ''}`,
        'POST',
        draft(),
      );
      if (test) message = 'Connection successful.';
      else {
        await added();
        selected = null;
        values = {};
        expanded = false;
      }
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
  function optionsValue(field: Field, event: Event) {
    const raw = (event.currentTarget as HTMLInputElement).value;
    values[field.name] =
      field.type === 'select'
        ? (field.selectOptions?.find((option) => String(option.value) === raw)
            ?.value ?? raw)
        : field.type === 'number'
          ? raw === ''
            ? null
            : Number(raw)
          : field.type === 'tagSelect'
            ? raw
                .split(',')
                .map((v) => v.trim())
                .filter(Boolean)
                .map((v) => (/^\d+$/.test(v) ? Number(v) : v))
            : raw;
  }
</script>

{#if !expanded}<Button
    variant="secondary"
    size="sm"
    class="mt-4"
    onclick={() => void start()}><Plus size={14} /> Add indexer</Button
  >
{:else}<section class="mt-4 border border-line p-4" aria-label="Add indexer">
    <div class="mb-4 flex items-center justify-between">
      <h4 class="text-sm font-semibold">
        {selected ? `Set up ${selected.name}` : 'Add indexer'}
      </h4>
      <Button
        variant="ghost"
        size="sm"
        disabled={busy}
        aria-label="Close indexer setup"
        onclick={() => {
          expanded = false;
          selected = null;
          values = {};
        }}><X size={15} /></Button
      >
    </div>
    {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
    {#if message}<p role="status" class="mb-3 text-xs text-accent">
        {message}
      </p>{/if}
    {#if loading}<p role="status" class="text-xs text-muted">
        Loading supported indexers…
      </p>{:else if selected}
      <Button
        variant="ghost"
        size="sm"
        class="mb-3"
        disabled={busy}
        onclick={() => {
          selected = null;
          values = {};
        }}><ArrowLeft size={14} /> Choose another indexer</Button
      >
      <form
        class="grid gap-3 [&_label]:m-0"
        onsubmit={(event) => {
          event.preventDefault();
          void submit(false);
        }}
      >
        <FormField
          >Name<input
            class={formControlClass}
            bind:value={name}
            required
            maxlength="100"
          /></FormField
        >
        <FormField
          >Sync profile<select
            class={formControlClass}
            bind:value={profile}
            required
            >{#each profiles as item (item.id)}<option value={item.id}
                >{item.name}</option
              >{/each}</select
          ></FormField
        >
        {#each selected.fields.filter((f) => f.type !== 'hidden' && f.hidden !== 'hidden' && (!f.advanced || advanced)) as field (field.name)}
          {#if field.type === 'info'}<p class="text-xs leading-6 text-muted">
              {clean(String(field.value ?? field.helpText ?? ''))}
            </p>
          {:else if field.type === 'checkbox'}<Switch
              checked={Boolean(values[field.name])}
              size="sm"
              onCheckedChange={(checked) => (values[field.name] = checked)}
              >{field.label}</Switch
            >
          {:else if field.type === 'select' && field.selectOptions?.length}<FormField
              >{field.label}<select
                class={formControlClass}
                value={String(values[field.name] ?? '')}
                onchange={(event) => optionsValue(field, event)}
                >{#each field.selectOptions as option (option.value)}<option
                    value={option.value}>{option.name}</option
                  >{/each}</select
              ></FormField
            >
          {:else if field.type === 'tagSelect' && field.selectOptions?.length}<fieldset
              class="grid grid-cols-2 gap-2"
            >
              <legend class="mb-2 text-xs text-muted">{field.label}</legend
              >{#each field.selectOptions as option (option.value)}<label
                  class="flex items-center gap-2 text-xs"
                  ><input
                    class="accent-accent"
                    type="checkbox"
                    checked={(
                      (values[field.name] as (number | string)[]) ?? []
                    ).includes(option.value)}
                    onchange={(event) => {
                      const old =
                        (values[field.name] as (number | string)[]) ?? [];
                      values[field.name] = event.currentTarget.checked
                        ? [...old, option.value]
                        : old.filter((v) => v !== option.value);
                    }}
                  />{option.name}</label
                >{/each}
            </fieldset>
          {:else}<FormField
              >{field.label}<input
                class={formControlClass}
                type={field.type === 'password' ||
                ['password', 'apiKey'].includes(field.privacy ?? '')
                  ? 'password'
                  : field.type === 'number'
                    ? 'number'
                    : 'text'}
                value={Array.isArray(values[field.name])
                  ? (values[field.name] as unknown[]).join(', ')
                  : String(values[field.name] ?? '')}
                onchange={(event) => optionsValue(field, event)}
                autocomplete="off"
              />{#if field.helpText}<span
                  class="mt-1 block text-[10px] leading-5 text-muted"
                  >{clean(field.helpText)}</span
                >{/if}</FormField
            >{/if}
          {#if field.type === 'cardigannCaptcha'}
            {#if captchaImages[field.name]}<img
                src={captchaImages[field.name]}
                alt="Indexer verification challenge"
                class="max-h-32 max-w-full object-contain"
              />{/if}
            <Button
              variant="secondary"
              size="sm"
              disabled={busy || fieldBusy !== null}
              onclick={() => void loadField(field)}
              >Load verification image</Button
            >
          {:else if field.selectOptionsProviderAction === 'getUrls' && !field.selectOptions?.length}
            <Button
              variant="ghost"
              size="sm"
              disabled={fieldBusy !== null}
              onclick={() => void loadField(field)}
              >{fieldBusy
                ? 'Loading addresses…'
                : 'Load supported addresses'}</Button
            >
          {/if}
        {/each}
        {#if selected.fields.some((f) => f.advanced)}<Switch
            bind:checked={advanced}
            size="sm">Advanced settings</Switch
          >{/if}
        <div class="mt-2 flex flex-wrap gap-2">
          <Button
            type="button"
            variant="secondary"
            size="form"
            disabled={busy || !profile}
            onclick={() => void submit(true)}
            >{busy ? 'Checking…' : 'Test connection'}</Button
          ><Button
            type="submit"
            size="form"
            disabled={busy || !profile || !name.trim()}>Add indexer</Button
          >
        </div>
      </form>
    {:else}
      <div class="mb-4 flex gap-2">
        <label class="relative min-w-0 flex-1"
          ><span class="sr-only">Find an indexer</span><Search
            size={15}
            class="absolute top-3 left-3 text-muted"
          /><input
            class={`${formControlClass} pl-9`}
            placeholder="Find an indexer…"
            bind:value={query}
          /></label
        ><select
          class={`${formControlClass} w-30`}
          aria-label="Indexer protocol"
          bind:value={protocol}
          ><option value="all">All types</option><option value="usenet"
            >Usenet</option
          ><option value="torrent">Torrent</option></select
        >
      </div>
      <p class="mb-3 text-[10px] text-muted">
        {filtered.length} supported indexers
      </p>
      <div
        class="grid max-h-96 grid-cols-2 gap-2 overflow-y-auto compact:grid-cols-1"
      >
        {#each filtered as definition (`${definition.implementation}:${definition.name}`)}<button
            class="border border-line p-3 text-left hover:border-accent focus-visible:outline-accent"
            onclick={() => choose(definition)}
            ><strong class="block text-xs">{definition.name}</strong><span
              class="mt-1 block text-[10px] text-muted"
              >{definition.protocol} · {definition.privacy}</span
            ></button
          >{/each}
      </div>
      {#if !filtered.length}<p class="py-4 text-xs text-muted">
          No indexers match your search.
        </p>{/if}
    {/if}
  </section>{/if}
