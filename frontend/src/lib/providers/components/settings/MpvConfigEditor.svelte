<script lang="ts">
  import Button from '../ui/Button.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';
  import Switch from '../ui/Switch.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { normalizeError } from '../../api';
  import {
    toolsApi,
    type MpvPreferences,
    type ConfigDocument,
    type MpvSchema,
  } from '../../tools-api';
  import {
    mpvConfigFiles,
    mpvDrafts,
    restoreMpvDraft,
    discardSavedMpvDraft,
  } from '../../mpv-config';
  import MpvOptionsForm from './MpvOptionsForm.svelte';
  import MpvConfigurationSource from './MpvConfigurationSource.svelte';
  import { toolsState } from '../../tools-state';
  import { loadMpvSchema } from '../../mpv-schema';
  import { LatestRequest } from '../../latest-request';
  import { displayToolPath } from '../../tools-presentation';
  import { controlClass } from './settingsUi';
  let {
    preferences,
    onNotice,
    onError,
    onChanged,
    initialFile = 'mpv.conf',
    request = 0,
    executableKey,
    directory,
    executableBusy = false,
  }: {
    preferences: MpvPreferences;
    onNotice: (text: string) => void;
    onError: (error: unknown) => void;
    onChanged: () => Promise<void>;
    initialFile?: string;
    request?: number;
    executableKey: string | null;
    directory: string;
    executableBusy?: boolean;
  } = $props();
  let busy = $state(false);
  let reading = $state(false);
  let activity = $state('');
  let importPath = $state<string | null>(null);
  let importError = $state('');
  const loads = new LatestRequest();
  const schemaLoads = new LatestRequest();
  let disposed = false;
  let files = $state<string[]>([
    'mpv.conf',
    'input.conf',
    'script-opts/uosc.conf',
    'script-opts/thumbfast.conf',
    'script-opts/sub_select.conf',
    'script-opts/sub-select.json',
  ]);
  let name = $state(untrack(() => initialFile));
  let document = $state<ConfigDocument | null>(null);
  let text = $state('');
  let revision = $state('');
  let schema = $state<MpvSchema | null>(null);
  let schemaError = $state('');
  let raw = $state(false);
  let search = $state('');
  let all = $state(false);
  let loading = $state(false);
  const configurationSource = $derived(preferences.source);
  const editorBusy = $derived(busy || reading);
  const rawVisible = $derived(raw || Boolean(schemaError && !schema));
  const conflict = $derived(
    document !== null && revision !== document.revision,
  );
  const dirty = $derived(
    document !== null && (text !== document.text || conflict),
  );
  function rememberDraft() {
    if (!document) return;
    if (dirty) mpvDrafts.set(document.name, { text, revision });
    else mpvDrafts.delete(document.name);
  }
  async function load(next = name) {
    if (disposed) return;
    rememberDraft();
    const current = loads.begin();
    reading = true;
    try {
      const [loaded, availableFiles] = await Promise.all([
        toolsApi.read(next),
        toolsApi.files(),
      ]);
      if (!current() || disposed || configurationSource !== 'managed') return;
      document = loaded;
      name = next;
      files = mpvConfigFiles(availableFiles, mpvDrafts);
      const draft = restoreMpvDraft(document, mpvDrafts.get(next));
      text = draft.text;
      revision = draft.revision;
      if (revision !== document.revision) raw = true;
      if (next === 'mpv.conf' && !raw && !schema && !loading && executableKey)
        void loadSchema();
    } catch (error) {
      if (current() && !disposed) throw error;
    } finally {
      if (current()) reading = false;
    }
  }
  function discard() {
    if (!document) return;
    text = document.text;
    revision = document.revision;
    mpvDrafts.delete(name);
  }
  async function operation(
    action: () => Promise<unknown>,
    message?: string,
    label = 'Saving configuration...',
    usesExecutable = false,
  ) {
    if (disposed) return false;
    busy = true;
    activity = label;
    try {
      if (!(await toolsState.settle('configuration'))) return false;
      if (usesExecutable && !(await toolsState.settle('mpv'))) return false;
      await action();
      toolsState.clearError('configuration');
      if (message) onNotice(message);
      return true;
    } catch (error) {
      onError(error);
      return false;
    } finally {
      busy = false;
      activity = '';
    }
  }
  async function loadSchema(refresh = false) {
    if (!executableKey || executableBusy) return;
    const key = executableKey;
    const current = schemaLoads.begin();
    loading = true;
    schemaError = '';
    try {
      const next = await loadMpvSchema(key, refresh, async () => {
        if (!(await toolsState.settle('mpv')) || key !== executableKey)
          throw new Error(
            'The selected MPV executable changed. Retry option discovery.',
          );
        const next = await toolsApi.schema();
        if (key !== executableKey)
          throw new Error(
            'The selected MPV executable changed. Retry option discovery.',
          );
        return next;
      });
      if (current() && !disposed && key === executableKey) schema = next;
    } catch (error) {
      if (current() && !disposed) {
        schemaError = normalizeError(error).message;
      }
    } finally {
      if (current()) loading = false;
    }
  }
  async function save(overwrite = false) {
    if (!document) return;
    const file = name;
    const submitted = text;
    const base = revision;
    const expected = overwrite ? document.revision : revision;
    await toolsState.trackSave(
      operation(
        async () => {
          let saved: ConfigDocument;
          try {
            saved = await toolsApi.save(file, submitted, expected);
          } catch (error) {
            // Refresh the disk comparison without losing the draft or its base.
            await load();
            throw error;
          }
          discardSavedMpvDraft(mpvDrafts, file, {
            text: submitted,
            revision: base,
          });
          if (!disposed && name === file) {
            document = saved;
            text = saved.text;
            revision = saved.revision;
          }
        },
        undefined,
        name === 'mpv.conf'
          ? 'Validating and saving MPV configuration...'
          : 'Saving configuration...',
        file === 'mpv.conf',
      ),
    );
  }
  async function restore() {
    if (!document) return;
    const file = name;
    const expected = document.revision;
    await toolsState.trackSave(
      operation(
        async () => {
          const restored = await toolsApi.restore(file, expected);
          if (!disposed && name === file) {
            document = restored;
            text = restored.text;
            revision = restored.revision;
          }
        },
        'Previous saved configuration restored.',
        'Restoring configuration...',
        file === 'mpv.conf',
      ),
    );
  }
  async function importConfiguration() {
    try {
      const path = await open({
        directory: true,
        multiple: false,
        title: 'Select a folder to copy into Thelxinoe',
      });
      if (typeof path !== 'string') return;
      importPath = path;
      importError = '';
    } catch (error) {
      onError(error);
    }
  }
  async function confirmImport() {
    if (!importPath) return;
    const path = importPath;
    await toolsState.trackSave(
      operation(
        async () => {
          const copied = await toolsState.run(
            'configuration',
            () => toolsApi.importConfig(path),
            'Copying configuration...',
          );
          if (!copied) {
            importError =
              $toolsState.errors.configuration ||
              'The folder could not be copied.';
            throw new Error(importError);
          }
          // Other files can still have unsaved drafts. Reconcile them against the
          // imported files when opened, just like any external disk edit.
          document = null;
          await load();
          await onChanged();
          importPath = null;
          onNotice(
            'Configuration copied. The previous managed folder was kept as a backup.',
          );
        },
        undefined,
        'Copying configuration...',
      ),
    );
  }
  async function addPlugin() {
    try {
      const path = await open({
        multiple: false,
        title: 'Add a trusted MPV script',
        filters: [{ name: 'MPV script', extensions: ['lua', 'js'] }],
      });
      if (typeof path !== 'string') return;
      await toolsState.trackSave(
        operation(
          async () => {
            await toolsApi.importPlugin(path);
            await onChanged();
            onNotice(
              'Local plugin copied into the managed configuration. Existing plugin selections are unchanged.',
            );
          },
          undefined,
          'Copying plugin...',
        ),
      );
    } catch (error) {
      onError(error);
    }
  }
  async function test(clean: boolean) {
    await operation(
      async () => {
        onNotice(await toolsApi.testConfiguration(clean));
      },
      undefined,
      'Testing MPV configuration...',
      true,
    );
  }
  $effect(() => {
    const source = configurationSource;
    const next = initialFile;
    void request;
    untrack(() => {
      rememberDraft();
      loads.invalidate();
      reading = false;
      if (source === 'managed') void load(next).catch(onError);
    });
  });
  $effect(() => {
    void executableKey;
    void executableBusy;
    untrack(() => {
      schemaLoads.invalidate();
      schema = null;
      schemaError = '';
      loading = false;
      if (preferences.source === 'managed' && name === 'mpv.conf' && !raw)
        void loadSchema();
    });
  });
  onDestroy(() => {
    rememberDraft();
    disposed = true;
    loads.invalidate();
    schemaLoads.invalidate();
  });
</script>

<section class="flex min-h-0 flex-1 flex-col gap-3" aria-busy={editorBusy}>
  <MpvConfigurationSource {preferences} {directory} {onError} />
  <p class="sr-only" role="status">
    {activity ||
      $toolsState.pending.configuration ||
      (reading ? 'Reading configuration...' : '')}
  </p>
  {#if preferences.source === 'managed'}
    <details class="shrink-0 border-t border-(--line) pt-2">
      <summary class="cursor-pointer text-xs font-medium text-(--muted)"
        >Copy files and restore backups</summary
      >
      <div class="mt-3 flex flex-wrap gap-2">
        <Button
          variant="secondary"
          size="sm"
          disabled={editorBusy || dirty}
          title={dirty
            ? 'Save or discard the current draft before replacing the folder.'
            : 'Copy a folder without changing its original files'}
          onclick={importConfiguration}>Copy a folder into Thelxinoe...</Button
        ><Button
          variant="secondary"
          size="sm"
          disabled={editorBusy}
          onclick={addPlugin}>Add a trusted local plugin...</Button
        >
      </div>
      {#if preferences.source === 'managed'}
        <div class="mt-3 flex flex-wrap gap-2">
          <Button
            variant="secondary"
            size="sm"
            disabled={editorBusy ||
              (name === 'mpv.conf' && (executableBusy || !executableKey)) ||
              !document?.hasBackup ||
              dirty}
            onclick={restore}>Restore previous save</Button
          >
          <Button
            variant="secondary"
            size="sm"
            disabled={editorBusy || !document}
            onclick={() =>
              operation(() => load(), undefined, 'Reading configuration...')}
            >Check disk for changes</Button
          >
          <Button
            variant="secondary"
            size="sm"
            disabled={loading || editorBusy || executableBusy || !executableKey}
            onclick={() => loadSchema(true)}>Refresh available options</Button
          >
        </div>
      {/if}
    </details>
  {/if}
  <div class="flex flex-wrap gap-2">
    <Button
      variant="secondary"
      size="sm"
      disabled={editorBusy ||
        executableBusy ||
        !executableKey ||
        (preferences.source === 'managed' && dirty)}
      onclick={() => test(false)}>Test selected configuration</Button
    >
    <Button
      variant="ghost"
      size="sm"
      disabled={editorBusy || executableBusy || !executableKey}
      onclick={() => test(true)}>Test MPV with clean defaults</Button
    >
  </div>
  {#if preferences.source === 'managed'}
    <div
      class="flex min-h-64 flex-1 flex-col gap-2 border-t border-(--line) pt-3"
    >
      <div class="flex flex-wrap items-end gap-2">
        <label class="grid w-full max-w-sm gap-1 text-xs"
          ><span class="sr-only">File</span><select
            class={controlClass}
            value={name}
            disabled={editorBusy}
            onchange={(event) => {
              const next = event.currentTarget.value;
              void operation(
                () => load(next),
                undefined,
                'Reading configuration...',
              );
            }}
            >{#each files as file (file)}<option value={file}
                >{displayToolPath(file)}</option
              >{/each}</select
          ></label
        >
        {#if name === 'mpv.conf'}<Switch
            size="sm"
            class="min-h-8 text-xs"
            checked={rawVisible}
            onCheckedChange={(checked) => {
              raw = checked;
              if (!raw && !schema && !loading) {
                schemaError = '';
                void loadSchema();
              }
            }}>Raw</Switch
          >{/if}
        <Button
          variant="secondary"
          size="sm"
          disabled={editorBusy ||
            (name === 'mpv.conf' && (executableBusy || !executableKey)) ||
            !dirty ||
            conflict}
          onclick={() => save()}>Save</Button
        >
      </div>
      {#if conflict && document}
        <div
          class="grid gap-2 rounded-md border border-(--warning) p-3 text-xs"
        >
          <p>
            The file changed on disk. Your draft is still in the editor. Compare
            the disk version below, then merge it into your draft, save your
            draft over it, or discard your draft.
          </p>
          <details>
            <summary class="cursor-pointer"
              >Disk version of {displayToolPath(name)}</summary
            >
            <textarea
              aria-label="Disk version of {displayToolPath(name)}"
              class={controlClass +
                ' mt-2 min-h-40 resize-y font-mono leading-6'}
              readonly
              value={document.text}></textarea>
          </details>
          <Button
            variant="secondary"
            size="sm"
            class="justify-self-start"
            disabled={editorBusy ||
              (name === 'mpv.conf' && (executableBusy || !executableKey))}
            onclick={() => save(true)}>Save draft over disk version</Button
          >
        </div>
      {/if}
      {#if dirty}<p class="text-xs text-(--warning)">
          Unsaved changes. Your draft is kept while navigating settings.
        </p>
        <Button
          variant="secondary"
          size="sm"
          class="justify-self-start"
          disabled={editorBusy}
          onclick={discard}
          >{conflict
            ? 'Discard draft and use disk version'
            : 'Discard changes'}</Button
        >{/if}
      {#if schemaError}<p class="text-xs text-(--warning)">
          {schemaError}
        </p>{/if}
      {#if name !== 'mpv.conf' || rawVisible}
        <label class="flex min-h-40 flex-1 flex-col gap-2 text-xs"
          ><span class="sr-only">{displayToolPath(name)}</span><textarea
            class={controlClass +
              ' min-h-64 flex-1 resize-y font-mono leading-5'}
            spellcheck="false"
            bind:value={text}
            disabled={editorBusy || !document}></textarea></label
        >
      {:else}
        {#if loading && !schema}<p class="text-xs text-(--muted)" role="status">
            Reading MPV options…
          </p>{/if}
        {#if schema}
          <MpvOptionsForm
            options={schema.options}
            bind:text
            bind:search
            bind:all
            disabled={editorBusy || !document}
            {onError}
          />
        {/if}
      {/if}
    </div>
  {/if}
</section>

<ConfirmDialog
  open={importPath !== null}
  title="Copy and use this MPV configuration?"
  eyebrow="MPV configuration"
  confirmLabel="Copy and use this configuration"
  {busy}
  error={importError}
  message={`Copy ${displayToolPath(importPath)} into ${displayToolPath(directory + '/config/mpv')}. Thelxinoe will use this separate copy for new players. Your original folder stays untouched, and later changes to it will not be synchronized. The existing managed folder will be backed up. Scripts in this folder run with your Windows account. The MPV executable selection stays unchanged.`}
  onConfirm={() => void confirmImport()}
  onCancel={() => (importPath = null)}
/>
