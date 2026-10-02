<script lang="ts">
  import { untrack, tick, onMount } from 'svelte';
  import {
    Maximize,
    Minimize,
    Search,
    WrapText,
    Undo2,
    Redo2,
    FilePlus,
    Pencil,
    Trash2,
    Download,
    Upload,
    Keyboard,
    ListFilter,
  } from '@lucide/svelte';
  import { api, ApiError } from '../api';
  import Modal from '../ui/Modal.svelte';
  import Button from '../ui/Button.svelte';
  import YamlEditor from '../ui/YamlEditor.svelte';
  import YamlDiff from '../ui/YamlDiff.svelte';
  import { formControlClass } from '../ui/styles';
  import { yamlProblems, yamlOutline } from '../ui/yaml-editor';
  import { schemaTools, type EditorAssistance } from '../ui/yaml-schema';
  import {
    sameFiles,
    yamlFilePath,
    type YamlFiles,
    type YamlProblem,
    type RecyclarrConfiguration,
  } from './recyclarr-configuration';

  let {
    initial,
    draft,
    candidate = false,
    onClose,
    onUpdate,
  } = $props<{
    initial: RecyclarrConfiguration;
    draft?: YamlFiles;
    candidate?: boolean;
    onClose: () => void;
    onUpdate: (configuration: RecyclarrConfiguration) => void;
  }>();
  let snapshot = $state<RecyclarrConfiguration>(untrack(() => initial));
  let savedFiles = $state<YamlFiles>(
    untrack(() => (candidate ? initial.candidate!.files : initial.files)),
  );
  let files = $state<YamlFiles>(untrack(() => ({ ...(draft ?? savedFiles) })));
  let file = $state(
    untrack(() =>
      'recyclarr.yml' in files ? 'recyclarr.yml' : Object.keys(files).sort()[0],
    ),
  );
  let customized = $state(
    untrack(() => candidate || initial.mode === 'customized' || !!draft),
  );
  let expanded = $state(false),
    wrap = $state(false),
    saving = $state(false),
    validating = $state(false);
  let error = $state(''),
    diagnostics = $state<YamlProblem[]>([]),
    preview = $state('');
  let status = $state({ line: 1, column: 1, errors: 0 });
  let discard = $state(false),
    shortcuts = $state(false),
    fileAction = $state<'new' | 'rename' | 'delete' | null>(null),
    filename = $state('');
  let actionError = $state(''),
    latest = $state<RecyclarrConfiguration | null>(null),
    compare = $state(false),
    reload = $state(false);
  let editor = $state<YamlEditor>();
  let upload: HTMLInputElement;
  let alive = true;
  let replacement = $state<{ name: string; text: string } | null>(null);
  let assistance = $state<ReturnType<typeof schemaTools> | null>(null);
  let schemaError = $state(''),
    schemaVersion = $state('');
  let schemaProblems = $state<YamlProblem[]>([]);
  const problems = $derived([...schemaProblems, ...diagnostics]);
  $effect(() => {
    const selected = files,
      tools = assistance;
    const timer = setTimeout(() => {
      schemaProblems = tools?.problems(selected) ?? [];
    }, 400);
    return () => clearTimeout(timer);
  });
  const dirty = $derived(!sameFiles(files, savedFiles));
  const outline = $derived(yamlOutline(files[file] ?? ''));
  const names = $derived(
    Object.keys(files).sort((a, b) =>
      a === 'recyclarr.yml'
        ? -1
        : b === 'recyclarr.yml'
          ? 1
          : a.localeCompare(b),
    ),
  );
  const syntaxErrors = $derived(
    Object.values(files).reduce(
      (sum, content) => sum + yamlProblems(content).length,
      0,
    ),
  );
  const completions = $derived([
    ...(assistance?.completions ?? []),
    ...snapshot.bindings.flatMap((binding) => [
      {
        label: binding.base_url,
        type: 'variable',
        detail: `${binding.kind} URL`,
      },
      {
        label: binding.api_key,
        type: 'variable',
        detail: `${binding.kind} API key`,
      },
      {
        label: `${binding.kind} instance`,
        type: 'snippet',
        detail: binding.instance,
        apply: `${binding.kind}:\n  ${binding.instance}:\n    base_url: !secret ${binding.base_url}\n    api_key: !secret ${binding.api_key}\n    quality_profiles: []\n`,
      },
    ]),
    ...Object.keys(files)
      .filter((name) => name.startsWith('includes/'))
      .map((name) => ({
        label: name.slice(9),
        type: 'text',
        detail: 'Local include',
      })),
  ]);
  onMount(() => {
    void api<EditorAssistance>(
      `/admin/recyclarr/configuration/editor?candidate=${candidate}`,
    )
      .then((value) => {
        if (!alive) return;
        assistance = schemaTools(value);
        schemaError = value.schema_error ?? '';
        schemaVersion = value.version ?? '';
      })
      .catch(() => {
        if (alive)
          schemaError =
            'Schema assistance is unavailable. Validate with Recyclarr before saving.';
      });
    const guard = (event: BeforeUnloadEvent) => {
      if (dirty || saving) event.preventDefault();
    };
    window.addEventListener('beforeunload', guard);
    return () => {
      alive = false;
      window.removeEventListener('beforeunload', guard);
    };
  });
  function replaceFiles(next: YamlFiles) {
    files = next;
    diagnostics = [];
    preview = '';
  }
  function changed(name: string, text: string) {
    replaceFiles({ ...files, [name]: text });
  }
  function close() {
    if (saving) {
      error = 'The save is still running. Your edits remain available.';
      return;
    }
    if (dirty) discard = true;
    else onClose();
  }
  async function save() {
    if (!customized || saving || syntaxErrors || !dirty) return;
    saving = true;
    error = '';
    latest = null;
    const submitted = { ...files };
    try {
      const result = await api<RecyclarrConfiguration>(
        `/admin/recyclarr/configuration${candidate ? '/candidate' : ''}`,
        candidate ? 'POST' : 'PUT',
        {
          revision: snapshot.revision,
          candidate_revision: candidate
            ? snapshot.candidate?.revision
            : undefined,
          files: submitted,
        },
      );
      onUpdate(result);
      if (alive) {
        snapshot = result;
        savedFiles = submitted;
      }
    } catch (caught) {
      if (alive) {
        error = String(caught);
        if (caught instanceof ApiError && caught.status === 409) {
          try {
            latest = await api<RecyclarrConfiguration>(
              '/admin/recyclarr/configuration',
            );
          } catch (refreshError) {
            error += ` ${String(refreshError)}`;
          }
        }
      }
    } finally {
      if (alive) saving = false;
    }
  }
  async function validate() {
    if (validating || syntaxErrors) return;
    validating = true;
    error = '';
    const submitted = { ...files };
    try {
      const result = await api<{
        valid: boolean;
        diagnostics: YamlProblem[];
        preview?: { preview?: string };
      }>('/admin/recyclarr/configuration/validate', 'POST', {
        revision: snapshot.revision,
        files: submitted,
      });
      if (alive && sameFiles(files, submitted)) {
        diagnostics = result.diagnostics;
        preview =
          result.preview?.preview ??
          (result.valid
            ? 'Configuration is valid.'
            : 'Resolve the listed problems.');
      }
    } catch (caught) {
      if (alive) error = String(caught);
    } finally {
      if (alive) validating = false;
    }
  }
  async function format() {
    if (!customized) return;
    const selected = file,
      original = files[file];
    try {
      const [{ format }, yaml] = await Promise.all([
        import('prettier/standalone'),
        import('prettier/plugins/yaml'),
      ]);
      const formatted = await format(original, {
        parser: 'yaml',
        plugins: [yaml],
        tabWidth: 2,
        printWidth: 100,
      });
      if (files[selected] === original) changed(selected, formatted);
    } catch (caught) {
      error = `Cannot format YAML: ${String(caught)}`;
    }
  }
  function changeFileAction(action: 'new' | 'rename' | 'delete') {
    fileAction = action;
    filename = action === 'new' ? 'includes/local.yml' : file;
    actionError = '';
  }
  function applyFileAction() {
    const name = filename.trim();
    if (
      fileAction !== 'delete' &&
      (!yamlFilePath(name) ||
        (name in files && (fileAction === 'new' || name !== file)))
    ) {
      actionError =
        'Choose a unique .yml or .yaml file under configs/ or includes/.';
      return;
    }
    if (fileAction === 'rename' && name === file) {
      fileAction = null;
      return;
    }
    if (fileAction === 'delete' && names.length === 1) {
      actionError = 'Keep at least one configuration file.';
      return;
    }
    const next = { ...files };
    if (fileAction === 'new') next[name] = '{}\n';
    if (fileAction === 'rename') {
      editor?.renameFile(name);
      next[name] = next[file];
      delete next[file];
    }
    if (fileAction === 'delete') delete next[file];
    replaceFiles(next);
    file = fileAction === 'delete' ? Object.keys(next)[0] : name;
    fileAction = null;
  }
  async function importFile(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const selected = input.files?.[0];
    if (!selected) return;
    if (selected.size > 128 * 1024) {
      error = 'Keep each YAML file below 128 KiB.';
      input.value = '';
      return;
    }
    const name =
      selected.name === 'settings.yml' || selected.name === 'recyclarr.yml'
        ? selected.name
        : `includes/${selected.name}`;
    if (!yamlFilePath(name)) {
      error = 'Choose a supported YAML filename.';
      input.value = '';
      return;
    }
    try {
      const text = await selected.text();
      if (!alive) return;
      if (name in files) replacement = { name, text };
      else {
        changed(name, text);
        file = name;
      }
    } catch (caught) {
      error = `Cannot read the YAML file: ${String(caught)}`;
    }
    input.value = '';
  }
  function download() {
    const url = URL.createObjectURL(
      new Blob([files[file]], { type: 'application/yaml;charset=utf-8' }),
    );
    const link = document.createElement('a');
    link.href = url;
    link.download = file.split('/').at(-1)!;
    link.click();
    URL.revokeObjectURL(url);
  }
  async function tabKey(event: KeyboardEvent, index: number) {
    const offset =
      event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!offset && !['Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    file =
      names[
        event.key === 'Home'
          ? 0
          : event.key === 'End'
            ? names.length - 1
            : (index + offset + names.length) % names.length
      ];
    await tick();
    document.getElementById(`yaml-tab-${file}`)?.focus();
  }
  async function jump(problem: YamlProblem) {
    file = problem.file;
    compare = false;
    await tick();
    editor?.jump(problem);
  }
  function reloadLatest() {
    if (!latest) return;
    if (candidate && !latest.candidate) {
      error =
        'This candidate is no longer available. Close the editor and review the current update.';
      reload = false;
      return;
    }
    snapshot = latest;
    savedFiles = candidate ? latest.candidate!.files : latest.files;
    replaceFiles({ ...savedFiles });
    customized = candidate || latest.mode === 'customized';
    file = Object.keys(files)[0];
    error = '';
    latest = null;
    reload = false;
    compare = false;
  }
</script>

<Modal
  title="Recyclarr YAML"
  size="editor"
  {expanded}
  onClose={close}
  closeLabel="Close YAML editor"
>
  {#snippet actions()}<Button
      variant="ghost"
      size="icon"
      aria-label={expanded ? 'Restore editor size' : 'Expand editor'}
      onclick={() => (expanded = !expanded)}
      >{#if expanded}<Minimize size={16} />{:else}<Maximize
          size={16}
        />{/if}</Button
    >{/snippet}
  <div
    class="flex shrink-0 items-center justify-between gap-3 border-b border-line px-4 py-2 text-[11px] text-muted"
  >
    <span
      >{candidate
        ? 'Candidate configuration'
        : customized
          ? 'Customized'
          : 'Defaults'}{#if dirty}
        · Unsaved changes{/if}</span
    ><span>{names.length} {names.length === 1 ? 'file' : 'files'}</span>
  </div>
  <div
    class="flex shrink-0 overflow-x-auto border-b border-line"
    role="tablist"
    aria-label="Configuration files"
  >
    {#each names as name, index (name)}<button
        id={`yaml-tab-${name}`}
        type="button"
        role="tab"
        aria-selected={file === name}
        aria-controls="yaml-file-panel"
        tabindex={file === name ? 0 : -1}
        class="shrink-0 border-r border-line px-4 py-2.5 text-xs hover:bg-surface-soft focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent {file ===
        name
          ? 'bg-accent-soft text-accent'
          : 'text-muted'}"
        onclick={() => {
          file = name;
        }}
        onkeydown={(event) => void tabKey(event, index)}
        >{name}{#if files[name] !== savedFiles[name]}<span
            class="ml-1 text-accent"
            aria-label="modified">*</span
          >{/if}</button
      >{/each}
  </div>
  <div
    class="flex shrink-0 flex-wrap items-center gap-1 border-b border-line px-3 py-1.5"
    role="toolbar"
    aria-label="YAML editor tools"
  >
    <select
      class={`${formControlClass} max-w-44 ${names.length > 6 ? '' : 'md:hidden'}`}
      aria-label="Choose YAML file"
      bind:value={file}
    >
      {#each names as name (name)}<option value={name}>{name}</option>{/each}
    </select>
    {#if outline.length}<select
        class={`${formControlClass} max-w-40`}
        aria-label="Go to instance or profile"
        value=""
        onchange={(event) => {
          const item = outline[Number(event.currentTarget.value)];
          if (item)
            void jump({
              file,
              line: item.line,
              column: item.column,
              message: '',
            });
          event.currentTarget.value = '';
        }}
        ><option value="">Outline</option
        >{#each outline as item, index (index)}<option value={index}
            >{item.label}</option
          >{/each}</select
      >{/if}
    <Button variant="ghost" size="sm" onclick={() => editor?.search()}
      ><Search size={14} />Find / Replace</Button
    >
    <Button
      variant="ghost"
      size="sm"
      disabled={!customized || compare}
      onclick={() => void format()}>Format</Button
    >
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="Undo"
      disabled={!customized || compare}
      onclick={() => editor?.history('undo')}><Undo2 size={15} /></Button
    >
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="Redo"
      disabled={!customized || compare}
      onclick={() => editor?.history('redo')}><Redo2 size={15} /></Button
    >
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="Wrap lines"
      aria-pressed={wrap}
      onclick={() => (wrap = !wrap)}><WrapText size={15} /></Button
    >
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="Go to line"
      onclick={() => editor?.line()}><ListFilter size={15} /></Button
    >
    <span class="mx-1 h-4 border-l border-line" aria-hidden="true"></span>
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="New YAML file"
      disabled={!customized}
      onclick={() => changeFileAction('new')}><FilePlus size={15} /></Button
    >
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="Rename YAML file"
      disabled={!customized || ['recyclarr.yml', 'settings.yml'].includes(file)}
      onclick={() => changeFileAction('rename')}><Pencil size={15} /></Button
    >
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="Delete YAML file"
      disabled={!customized}
      onclick={() => changeFileAction('delete')}><Trash2 size={15} /></Button
    >
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="Import YAML file"
      disabled={!customized}
      onclick={() => upload.click()}><Upload size={15} /></Button
    >
    <input
      bind:this={upload}
      type="file"
      accept=".yml,.yaml"
      class="hidden"
      onchange={(event) => void importFile(event)}
    />
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="Download current YAML file"
      onclick={download}><Download size={15} /></Button
    >
    <Button
      variant="ghost"
      size="compact-icon"
      aria-label="Keyboard shortcuts"
      onclick={() => (shortcuts = true)}><Keyboard size={15} /></Button
    >
  </div>
  {#if !customized}<p
      class="shrink-0 border-b border-line bg-accent-soft px-4 py-2 text-xs text-muted"
    >
      Defaults update with Recyclarr upgrades.
    </p>{/if}
  {#if error}<div
      class="shrink-0 border-b border-line px-4 py-2 text-xs text-danger"
      role="alert"
    >
      {error}{#if latest}<div class="mt-2 flex gap-2">
          <Button
            size="sm"
            variant="secondary"
            onclick={() => (compare = !compare)}
            >{compare ? 'Back to editor' : 'Compare latest'}</Button
          ><Button size="sm" variant="secondary" onclick={() => (reload = true)}
            >Reload latest files</Button
          >
        </div>{/if}
    </div>{/if}
  {#if schemaError}<p
      class="shrink-0 border-b border-line px-4 py-2 text-xs text-muted"
      role="status"
    >
      {schemaError}
    </p>{/if}
  <div
    id="yaml-file-panel"
    role="tabpanel"
    aria-labelledby={`yaml-tab-${file}`}
    class="min-h-0 min-w-0 flex-1 overflow-hidden"
  >
    {#if compare && latest}<YamlDiff
        before={files[file] ?? ''}
        after={(candidate
          ? latest.candidate?.files[file]
          : latest.files[file]) ?? ''}
        beforeLabel="Your draft"
        afterLabel="Latest saved file"
      />{:else}<YamlEditor
        bind:this={editor}
        {files}
        {file}
        readonly={!customized}
        {wrap}
        {completions}
        {problems}
        documentation={assistance?.documentation}
        onChange={changed}
        onStatus={(value) => {
          if (
            value.line !== status.line ||
            value.column !== status.column ||
            value.errors !== status.errors
          )
            status = value;
        }}
        onSave={() => void save()}
        onFormat={() => void format()}
      />{/if}
  </div>
  {#if problems.length}<div
      class="max-h-32 shrink-0 overflow-y-auto border-t border-line px-4 py-2"
      aria-label="YAML problems"
    >
      {#each problems as problem, index (index)}<button
          type="button"
          class="block w-full py-1 text-left text-xs whitespace-pre-wrap text-danger hover:underline"
          onclick={() => void jump(problem)}
          >{problem.file}:{problem.line}:{problem.column} — {problem.message}</button
        >{/each}
    </div>{/if}
  {#if preview}<details
      open
      class="max-h-40 shrink-0 overflow-auto border-t border-line px-4 py-2 text-xs"
    >
      <summary class="cursor-pointer text-muted">Validation preview</summary>
      <pre class="mt-2 text-[11px] whitespace-pre-wrap">{preview}</pre>
    </details>{/if}
  <div
    class="flex shrink-0 items-center justify-between gap-3 border-t border-line px-4 py-1.5 text-[10px] text-muted"
  >
    <span>Ln {status.line}, Col {status.column}</span><span
      >{syntaxErrors + problems.length
        ? `${syntaxErrors + problems.length} problems · `
        : ''}YAML{schemaVersion ? ` · ${schemaVersion}` : ''} · UTF-8 · 2 spaces</span
    >
  </div>
  <footer
    class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-t border-line px-4 py-3"
  >
    <span class="text-[11px] text-muted" aria-live="polite"
      >{saving
        ? 'Saving configuration…'
        : validating
          ? 'Validating with Recyclarr…'
          : candidate
            ? 'Candidate changes apply with the upgrade.'
            : 'Changes apply on the next sync.'}</span
    >
    <div class="flex gap-2">
      {#if customized}{#if !candidate}<Button
            variant="secondary"
            size="form"
            disabled={syntaxErrors > 0}
            loading={validating}
            onclick={() => void validate()}>Validate & preview</Button
          >{/if}<Button
          size="form"
          disabled={!dirty || syntaxErrors > 0}
          loading={saving}
          onclick={() => void save()}
          >{candidate ? 'Save candidate' : 'Save'}</Button
        >{:else}<Button
          size="form"
          onclick={() => {
            customized = true;
            editor?.focus();
          }}>Customize</Button
        >{/if}
    </div>
  </footer>
</Modal>

{#if replacement}<Modal
    title="Replace file?"
    onClose={() => (replacement = null)}
  >
    <p class="mb-5 text-sm">
      Replace the draft of {replacement.name} with the imported file?
    </p>
    <div class="flex justify-end gap-2">
      <Button variant="secondary" onclick={() => (replacement = null)}
        >Cancel</Button
      ><Button
        onclick={() => {
          if (!replacement) return;
          changed(replacement.name, replacement.text);
          file = replacement.name;
          replacement = null;
        }}>Replace file</Button
      >
    </div>
  </Modal>{/if}

{#if discard}<Modal title="Discard changes?" onClose={() => (discard = false)}
    ><p class="mb-5 text-sm">The file set has unsaved changes.</p>
    <div class="flex justify-end gap-2">
      <Button variant="secondary" onclick={() => (discard = false)}
        >Keep editing</Button
      ><Button variant="danger" onclick={onClose}>Discard changes</Button>
    </div></Modal
  >{/if}
{#if reload}<Modal title="Reload latest files?" onClose={() => (reload = false)}
    ><p class="mb-5 text-sm">
      This replaces your draft with the latest saved file set.
    </p>
    <div class="flex justify-end gap-2">
      <Button variant="secondary" onclick={() => (reload = false)}
        >Keep editing</Button
      ><Button onclick={reloadLatest}>Reload latest files</Button>
    </div></Modal
  >{/if}
{#if fileAction}<Modal
    title={fileAction === 'delete'
      ? 'Delete YAML file?'
      : fileAction === 'rename'
        ? 'Rename YAML file'
        : 'New YAML file'}
    onClose={() => (fileAction = null)}
  >
    {#if fileAction === 'delete'}<p class="mb-4 text-sm">
        Delete {file} from this draft?
      </p>{:else}<label class="mb-4 grid gap-2 text-xs"
        >File path<input
          class={formControlClass}
          bind:value={filename}
          onkeydown={(event) => {
            if (event.key === 'Enter') applyFileAction();
          }}
        /></label
      >{/if}
    {#if actionError}<p class="mb-3 text-xs text-danger" role="alert">
        {actionError}
      </p>{/if}
    <div class="flex justify-end gap-2">
      <Button variant="secondary" onclick={() => (fileAction = null)}
        >Cancel</Button
      ><Button
        variant={fileAction === 'delete' ? 'danger' : 'default'}
        onclick={applyFileAction}
        >{fileAction === 'new'
          ? 'Create file'
          : fileAction === 'rename'
            ? 'Rename file'
            : 'Delete file'}</Button
      >
    </div>
  </Modal>{/if}
{#if shortcuts}<Modal
    title="Keyboard shortcuts"
    onClose={() => (shortcuts = false)}
    ><dl class="grid grid-cols-[1fr_auto] gap-3 text-xs">
      <dt>Save</dt>
      <dd>Ctrl / ⌘ S</dd>
      <dt>Find / Replace</dt>
      <dd>Ctrl / ⌘ F</dd>
      <dt>Undo / Redo</dt>
      <dd>Ctrl / ⌘ Z / Shift Z</dd>
      <dt>Completion</dt>
      <dd>Ctrl Space</dd>
      <dt>Format</dt>
      <dd>Alt Shift F</dd>
      <dt>Go to line</dt>
      <dd>Alt G</dd>
      <dt>Toggle comment</dt>
      <dd>Ctrl / ⌘ /</dd>
      <dt>Let Tab leave editor</dt>
      <dd>Ctrl M</dd>
      <dt>Multiple cursors</dt>
      <dd>Alt click</dd>
    </dl></Modal
  >{/if}
