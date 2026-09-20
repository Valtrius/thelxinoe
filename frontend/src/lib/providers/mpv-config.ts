import type { ConfigDocument, MpvOption } from './tools-api';

// Only plain, global assignments are editable in the form. All other syntax stays raw.
interface Assignment {
  line: number;
  value: string;
  comment: string;
}
export function parseMpvConfig(
  text: string,
  options: readonly MpvOption[] = [],
) {
  const lines = text.split(/\r?\n/);
  const values = new Map<string, Assignment[]>();
  const blocked = new Set<string>();
  let section = lines.findIndex((line) => /^\s*\[/.test(line));
  if (section < 0) section = lines.length;
  const optionInfo = new Map(options.map((option) => [option.name, option]));
  let global = true;
  let complex = lines.some((line) => /%\d+%/.test(line) || /\\\s*$/.test(line));
  for (let i = 0; i < lines.length; i++) {
    if (/^\s*\[/.test(lines[i])) {
      const header = lines[i].match(/^\s*\[([^\]]*)\]\s*(?:#.*)?$/);
      global = header?.[1] === 'default';
      if (!header) complex = true;
      continue;
    }
    if (!global) continue;
    const match = lines[i].match(/^\s*([\w-]+)(?:\s*=\s*(.*)|(\s*(?:#.*)?))$/);
    if (!match || lines[i].trimStart().startsWith('#')) continue;
    const [, spelling, assignment, bareComment = ''] = match;
    let name = spelling.replace(/^--/, '');
    const negated = name.startsWith('no-') && !optionInfo.has(name);
    if (negated) {
      const canonical = name.slice(3);
      const option = optionInfo.get(canonical);
      if (option?.type !== 'Flag' && !option?.choices?.includes('no')) {
        // Unknown no- names may be real options; don't guess their semantics.
        blocked.add(name);
        blocked.add(canonical);
        continue;
      }
      name = canonical;
    }
    const raw = assignment ?? `yes${bareComment}`;
    const simple = raw.match(/^([^#"'\r\n]*?)(\s+#.*|#.*)?$/);
    if (
      !simple ||
      (negated && assignment !== undefined && simple[1].trim() !== '')
    ) {
      blocked.add(name);
      continue;
    }
    const entry = {
      line: i,
      value: negated ? 'no' : simple[1].trim(),
      comment: simple[2] ?? '',
    };
    values.set(name, [...(values.get(name) ?? []), entry]);
  }
  return {
    lines,
    values,
    blocked,
    section,
    complex,
    newline: text.includes('\r\n') ? '\r\n' : '\n',
  };
}
export function setMpvOption(
  text: string,
  name: string,
  value: string | null,
  options: readonly MpvOption[] = [],
): string {
  if (!/^[\w-]+$/.test(name) || (value !== null && /[\r\n\0#"']/.test(value)))
    throw new Error('Use the raw editor for quoted or multiline values.');
  const parsed = parseMpvConfig(text, options);
  if (parsed.complex || parsed.blocked.has(name))
    throw new Error('Use the raw editor for this configuration syntax.');
  const matches = parsed.values.get(name) ?? [];
  const last = matches.at(-1);
  const remove = new Set(matches.map((entry) => entry.line));
  const lines = parsed.lines.flatMap((line, index) => {
    if (!remove.has(index)) return [line];
    if (value !== null && index === last?.line)
      return [`${name}=${value}${last.comment}`];
    const comment = matches
      .find((entry) => entry.line === index)
      ?.comment.trimStart();
    return comment ? [comment] : [];
  });
  if (!last && value !== null) {
    let section = lines.findIndex((line) => /^\s*\[/.test(line));
    if (section < 0) section = lines.length;
    if (section > 0 && lines[section - 1] === '') section--;
    lines.splice(section, 0, `${name}=${value}`);
  }
  return lines.join(parsed.newline);
}

export interface MpvDraft {
  text: string;
  revision: string;
}

export function restoreMpvDraft(
  document: ConfigDocument,
  draft?: MpvDraft,
): MpvDraft {
  // A conflict keeps the original revision so an ordinary save cannot overwrite
  // external edits. An identical disk copy already contains the whole draft.
  return draft && draft.text !== document.text
    ? { ...draft }
    : { text: document.text, revision: document.revision };
}

export const mpvDrafts = new Map<string, MpvDraft>();

export function discardSavedMpvDraft(
  drafts: Map<string, MpvDraft>,
  name: string,
  submitted: MpvDraft,
) {
  const current = drafts.get(name);
  if (
    current?.text === submitted.text &&
    current.revision === submitted.revision
  )
    drafts.delete(name);
}

export function mpvConfigFiles(
  files: string[],
  drafts: ReadonlyMap<string, MpvDraft>,
): string[] {
  return [...new Set([...files, ...drafts.keys()])];
}
