<script lang="ts">
  import { onMount } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { basicSetup } from 'codemirror';
  import { yaml } from '@codemirror/lang-yaml';
  import { indentUnit } from '@codemirror/language';
  import { Compartment, EditorState } from '@codemirror/state';
  import { EditorView, keymap, hoverTooltip } from '@codemirror/view';
  import { undo, redo, indentWithTab } from '@codemirror/commands';
  import { openSearchPanel, gotoLine } from '@codemirror/search';
  import { autocompletion, type Completion } from '@codemirror/autocomplete';
  import { linter, setDiagnostics } from '@codemirror/lint';
  import { yamlProblems, yamlTheme } from './yaml-editor';
  import type {
    YamlFiles,
    YamlProblem,
  } from '../services/recyclarr-configuration';

  let {
    files,
    file,
    readonly = false,
    wrap = false,
    completions = [],
    problems = [],
    documentation = new Map<string, string>(),
    onChange,
    onStatus,
    onSave,
    onFormat,
  } = $props<{
    files: YamlFiles;
    file: string;
    readonly?: boolean;
    wrap?: boolean;
    completions?: Completion[];
    problems?: YamlProblem[];
    documentation?: Map<string, string>;
    onChange: (file: string, text: string) => void;
    onStatus: (status: {
      line: number;
      column: number;
      errors: number;
    }) => void;
    onSave: () => void;
    onFormat: () => void;
  }>();
  let container: HTMLDivElement;
  let view = $state<EditorView | null>(null);
  const states = new SvelteMap<string, EditorState>();
  const access = new Compartment(),
    wrapping = new Compartment();
  let current = '';
  const keywords = [
    'radarr',
    'sonarr',
    'include',
    'template',
    'config',
    'base_url',
    'api_key',
    'quality_profiles',
    'trash_id',
    'name',
    'upgrade',
    'allowed',
    'until_quality',
    'until_score',
    'min_format_score',
    'reset_unmatched_scores',
    'enabled',
    'except',
    'qualities',
    'custom_formats',
    'trash_ids',
    'assign_scores_to',
    'score',
    'custom_format_groups',
    'add',
    'skip',
    'quality_definition',
    'type',
    'preferred_ratio',
    'delete_old_custom_formats',
    'replace_existing_custom_formats',
    'media_naming',
    'media_management',
    'resource_providers',
    'replace_default',
    'path',
    'notifications',
    'log_janitor',
    'max_files',
    'url',
    'branch',
    'clone_url',
  ];
  function createState(text: string) {
    return EditorState.create({
      doc: text,
      extensions: [
        basicSetup,
        yaml(),
        EditorState.tabSize.of(2),
        indentUnit.of('  '),
        yamlTheme,
        access.of([
          EditorState.readOnly.of(readonly),
          EditorView.editable.of(!readonly),
        ]),
        wrapping.of(wrap ? EditorView.lineWrapping : []),
        hoverTooltip((editor, position) => {
          const line = editor.state.doc.lineAt(position);
          const match = /^\s*([\w-]+):/.exec(line.text);
          if (!match) return null;
          const start = line.from + line.text.indexOf(match[1]);
          const content = documentation.get(match[1]);
          if (
            !content ||
            position < start ||
            position > start + match[1].length
          )
            return null;
          return {
            pos: start,
            end: start + match[1].length,
            above: true,
            create: () => {
              const dom = document.createElement('div');
              dom.className = 'max-w-sm p-3 text-xs';
              dom.textContent = content;
              return { dom };
            },
          };
        }),
        EditorView.contentAttributes.of({
          'aria-label': 'YAML source',
          spellcheck: 'false',
        }),
        keymap.of([
          { key: 'Mod-Shift-z', run: redo, preventDefault: true },
          {
            key: 'Mod-s',
            run: () => {
              onSave();
              return true;
            },
          },
          {
            key: 'Alt-Shift-f',
            run: () => {
              onFormat();
              return true;
            },
          },
          indentWithTab,
        ]),
        autocompletion({
          override: [
            (context) => {
              const word = context.matchBefore(/[\w!./-]*/);
              if (!word || (!context.explicit && !word.text)) return null;
              return {
                from: word.from,
                options: [
                  ...(documentation.size
                    ? []
                    : keywords.map((label) => ({ label, type: 'property' }))),
                  ...completions,
                ],
              };
            },
          ],
        }),
        linter((editor) => yamlProblems(editor.state.doc.toString()), {
          delay: 350,
        }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) onChange(current, update.state.doc.toString());
          if (update.docChanged || update.selectionSet) status();
        }),
      ],
    });
  }
  function status() {
    if (!view) return;
    const position = view.state.selection.main.head,
      line = view.state.doc.lineAt(position);
    onStatus({
      line: line.number,
      column: position - line.from + 1,
      errors: yamlProblems(view.state.doc.toString()).length,
    });
  }
  onMount(() => {
    current = file;
    view = new EditorView({
      parent: container,
      state: createState(files[file] ?? ''),
    });
    status();
    return () => {
      view?.destroy();
      states.clear();
    };
  });
  $effect(() => {
    const editor = view,
      text = files[file] ?? '';
    if (!editor) return;
    if (file !== current) {
      states.set(current, editor.state);
      current = file;
      editor.setState(states.get(file) ?? createState(text));
    }
    if (editor.state.doc.toString() !== text)
      editor.dispatch({
        changes: { from: 0, to: editor.state.doc.length, insert: text },
      });
    editor.dispatch({
      effects: [
        access.reconfigure([
          EditorState.readOnly.of(readonly),
          EditorView.editable.of(!readonly),
        ]),
        wrapping.reconfigure(wrap ? EditorView.lineWrapping : []),
      ],
    });
    status();
  });
  $effect(() => {
    if (!view) return;
    const diagnostics = problems
      .filter((problem: YamlProblem) => problem.file === file)
      .map((problem: YamlProblem) => {
        const line = view!.state.doc.line(
          Math.min(Math.max(1, problem.line), view!.state.doc.lines),
        );
        const from = Math.min(
          line.to,
          line.from + Math.max(0, problem.column - 1),
        );
        return {
          from,
          to: Math.min(view!.state.doc.length, from + 1),
          severity: 'error' as const,
          message: problem.message,
        };
      });
    view.dispatch(
      setDiagnostics(view.state, [
        ...yamlProblems(view.state.doc.toString()),
        ...diagnostics,
      ]),
    );
  });
  export function focus() {
    view?.focus();
  }
  export function renameFile(name: string) {
    if (view) states.set(name, view.state);
  }
  export function search(replace = false) {
    if (!view) return;
    openSearchPanel(view);
    if (replace)
      container
        .querySelector<HTMLInputElement>('input[name="replace"]')
        ?.focus();
  }
  export function history(direction: 'undo' | 'redo') {
    if (view && !readonly) (direction === 'undo' ? undo : redo)(view);
    view?.focus();
  }
  export function line() {
    if (view) gotoLine(view);
  }
  export function jump(problem: YamlProblem) {
    if (!view) return;
    const line = view.state.doc.line(
      Math.min(problem.line, view.state.doc.lines),
    );
    view.dispatch({
      selection: {
        anchor: line.from + Math.min(problem.column - 1, line.length),
      },
      scrollIntoView: true,
    });
    view.focus();
  }
</script>

<div
  bind:this={container}
  class="h-full min-h-0 min-w-0 overflow-hidden [&_.cm-editor]:h-full"
  data-yaml-editor
></div>
