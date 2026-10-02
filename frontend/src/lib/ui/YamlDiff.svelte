<script lang="ts">
  import { onMount } from 'svelte';
  import { MergeView } from '@codemirror/merge';
  import { EditorState } from '@codemirror/state';
  import { EditorView } from '@codemirror/view';
  import { basicSetup } from 'codemirror';
  import { yaml } from '@codemirror/lang-yaml';
  import { yamlTheme } from './yaml-editor';
  let {
    before,
    after,
    beforeLabel = 'Active configuration',
    afterLabel = 'New defaults',
  } = $props<{
    before: string;
    after: string;
    beforeLabel?: string;
    afterLabel?: string;
  }>();
  let container: HTMLDivElement;
  let merge = $state<MergeView | null>(null);
  const extensions = [
    basicSetup,
    yaml(),
    yamlTheme,
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
  ];
  onMount(() => {
    merge = new MergeView({
      parent: container,
      a: { doc: before, extensions },
      b: { doc: after, extensions },
      highlightChanges: true,
      gutter: true,
    });
    return () => merge?.destroy();
  });
  $effect(() => {
    if (!merge) return;
    for (const [editor, text] of [
      [merge.a, before],
      [merge.b, after],
    ] as const) {
      if (editor.state.doc.toString() !== text)
        editor.dispatch({
          changes: { from: 0, to: editor.state.doc.length, insert: text },
        });
    }
  });
</script>

<div class="flex h-full min-h-0 flex-col overflow-hidden">
  <div
    class="grid shrink-0 grid-cols-2 border-b border-line text-[11px] text-muted compact:hidden"
  >
    <span class="px-4 py-2">{beforeLabel}</span><span
      class="border-l border-line px-4 py-2">{afterLabel}</span
    >
  </div>
  <div
    bind:this={container}
    class="min-h-0 flex-1 overflow-hidden [&_.cm-mergeView]:h-full [&_.cm-mergeViewEditors]:h-full [&_.cm-mergeViewEditors]:overflow-hidden [&_.cm-mergeViewEditor]:min-w-0 [&_.cm-mergeViewEditor]:overflow-hidden [&_.cm-mergeViewEditor]:border-line [&_.cm-editor]:h-full [&_.cm-deletedChunk]:bg-danger/10 [&_.cm-insertedChunk]:bg-success/10 [&_.cm-deletedText]:bg-danger/15 [&_.cm-insertedText]:bg-success/15 compact:[&_.cm-mergeViewEditors]:flex-col compact:[&_.cm-mergeViewEditor]:h-1/2 compact:[&_.cm-mergeViewEditor]:w-full compact:[&_.cm-mergeViewEditor]:border-b compact:[&_.cm-mergeViewEditor]:border-line compact:[&_.cm-mergeViewEditor:first-child]:before:block compact:[&_.cm-mergeViewEditor:first-child]:before:px-4 compact:[&_.cm-mergeViewEditor:first-child]:before:py-1 compact:[&_.cm-mergeViewEditor:first-child]:before:text-[11px] compact:[&_.cm-mergeViewEditor:first-child]:before:text-muted compact:[&_.cm-mergeViewEditor:first-child]:before:content-[var(--before-label)] compact:[&_.cm-mergeViewEditor:last-child]:before:block compact:[&_.cm-mergeViewEditor:last-child]:before:px-4 compact:[&_.cm-mergeViewEditor:last-child]:before:py-1 compact:[&_.cm-mergeViewEditor:last-child]:before:text-[11px] compact:[&_.cm-mergeViewEditor:last-child]:before:text-muted compact:[&_.cm-mergeViewEditor:last-child]:before:content-[var(--after-label)] compact:[&_.cm-editor]:h-[calc(100%-24px)]"
    style:--before-label={JSON.stringify(beforeLabel)}
    style:--after-label={JSON.stringify(afterLabel)}
  ></div>
</div>
