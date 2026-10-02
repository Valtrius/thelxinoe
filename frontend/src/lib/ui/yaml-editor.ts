import { EditorView } from '@codemirror/view';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { parseDocument, isMap, isSeq, isScalar, LineCounter } from 'yaml';

export const yamlTags = ['!secret', '!file', '!env_var'].map((tag) => ({
  tag,
  resolve: (value: string) => value,
}));
export function yamlProblems(text: string) {
  const document = parseDocument(text, {
    customTags: yamlTags,
    uniqueKeys: true,
  });
  return document.errors.map((error) => ({
    from: error.pos[0],
    to: error.pos[1],
    severity: 'error' as const,
    message: error.message,
  }));
}
export function yamlOutline(text: string) {
  const counter = new LineCounter();
  const document = parseDocument(text, {
    customTags: yamlTags,
    lineCounter: counter,
  });
  const items: { label: string; line: number; column: number }[] = [];
  for (const kind of ['radarr', 'sonarr']) {
    const instances = document.get(kind, true);
    if (!isMap(instances)) continue;
    for (const instance of instances.items) {
      if (!isScalar(instance.key)) continue;
      const position = counter.linePos(instance.key.range?.[0] ?? 0);
      items.push({
        label: `${kind} / ${instance.key.value}`,
        line: position.line,
        column: position.col,
      });
      if (!isMap(instance.value)) continue;
      const profiles = instance.value.get('quality_profiles', true);
      if (!isSeq(profiles)) continue;
      for (const profile of profiles.items) {
        if (!isMap(profile)) continue;
        const name = profile.get('name', true) ?? profile.get('trash_id', true);
        if (!isScalar(name)) continue;
        const position = counter.linePos(name.range?.[0] ?? 0);
        items.push({
          label: String(name.value),
          line: position.line,
          column: position.col,
        });
      }
    }
  }
  return items;
}
export const yamlTheme = [
  EditorView.theme({
    '&': {
      height: '100%',
      backgroundColor: 'var(--surface-strong)',
      color: 'var(--foreground)',
      fontSize: '12px',
    },
    '.cm-scroller': {
      overflow: 'auto',
      fontFamily: 'ui-monospace, SFMono-Regular, Consolas, monospace',
      lineHeight: '1.7',
    },
    '.cm-content': { caretColor: 'var(--accent)', padding: '12px 0' },
    '.cm-gutters': {
      backgroundColor: 'var(--surface-strong)',
      color: 'var(--muted)',
      borderColor: 'var(--line)',
    },
    '.cm-activeLine, .cm-activeLineGutter': {
      backgroundColor: 'var(--accent-soft)',
    },
    '.cm-selectionBackground, &.cm-focused .cm-selectionBackground, .cm-content ::selection':
      { backgroundColor: 'color-mix(in srgb, var(--accent) 24%, transparent)' },
    '&.cm-focused': { outline: 'none' },
    '.cm-panels, .cm-tooltip': {
      color: 'var(--foreground)',
      backgroundColor: 'var(--surface-strong)',
      borderColor: 'var(--line-strong)',
    },
    '.cm-search': {
      display: 'flex',
      flexWrap: 'wrap',
      gap: '6px',
      padding: '8px',
    },
    '.cm-textfield, .cm-button': {
      color: 'var(--foreground)',
      background: 'var(--surface-soft)',
      border: '1px solid var(--line-strong)',
      borderRadius: '0',
      fontSize: '12px',
    },
    '.cm-textfield:focus-visible, .cm-button:focus-visible': {
      outline: '2px solid var(--accent)',
      outlineOffset: '1px',
    },
    '.cm-tooltip-autocomplete > ul > li[aria-selected]': {
      background: 'var(--accent-soft)',
      color: 'var(--foreground)',
    },
    '.cm-matchingBracket': {
      backgroundColor: 'var(--accent-soft)',
      outline: '1px solid var(--line-strong)',
    },
    '.cm-searchMatch': {
      backgroundColor: 'color-mix(in srgb, var(--warning) 20%, transparent)',
    },
    '.cm-searchMatch-selected': { outline: '1px solid var(--warning)' },
    '.cm-diagnostic-error': { borderColor: 'var(--danger)' },
    '.cm-foldPlaceholder': {
      backgroundColor: 'var(--surface-soft)',
      color: 'var(--muted)',
      borderColor: 'var(--line)',
    },
  }),
  syntaxHighlighting(
    HighlightStyle.define([
      {
        tag: [tags.propertyName, tags.definition(tags.variableName)],
        color: 'var(--accent)',
      },
      {
        tag: [tags.string, tags.special(tags.string)],
        color: 'var(--success)',
      },
      { tag: [tags.number, tags.bool, tags.null], color: 'var(--warning)' },
      { tag: [tags.comment, tags.meta], color: 'var(--muted)' },
      {
        tag: [tags.keyword, tags.typeName, tags.tagName],
        color: 'var(--accent)',
        fontWeight: '600',
      },
    ]),
  ),
];
