import { Validator } from '@cfworker/json-schema';
import { parseDocument, isNode, LineCounter } from 'yaml';
import type { Completion } from '@codemirror/autocomplete';
import { yamlTags } from './yaml-editor';
import type {
  YamlFiles,
  YamlProblem,
} from '../services/recyclarr-configuration';

export type EditorAssistance = {
  version: string | null;
  revision: string | null;
  documents?: Record<string, object>;
  catalog: Completion[];
  schema_error?: string;
};
const base = 'https://recyclarr.invalid/schema/';
export function schemaTools(assistance: EditorAssistance) {
  const validators = new Map<string, Validator>();
  const completions = new Map<string, Completion>();
  const documentation = new Map<string, string>();
  for (const main of ['config-schema.json', 'settings-schema.json']) {
    if (!assistance.documents?.[main]) continue;
    const validator = new Validator(
      { ...assistance.documents[main], $id: base + main },
      '7',
      false,
    );
    for (const [name, document] of Object.entries(assistance.documents))
      if (name !== main) validator.addSchema({ ...document, $id: base + name });
    validators.set(main, validator);
  }
  function collect(value: unknown) {
    if (!value || typeof value !== 'object') return;
    const node = value as Record<string, unknown>;
    if (node.properties && typeof node.properties === 'object') {
      for (const [key, property] of Object.entries(node.properties)) {
        const detail = property as { description?: string; default?: unknown };
        completions.set(key, {
          label: key,
          type: 'property',
          info: detail.description,
        });
        if (detail.description) documentation.set(key, detail.description);
      }
    }
    if (Array.isArray(node.enum))
      for (const value of node.enum) {
        if (typeof value === 'string')
          completions.set(value, { label: value, type: 'enum' });
      }
    for (const child of Object.values(node)) collect(child);
  }
  for (const document of Object.values(assistance.documents ?? {}))
    collect(document);
  for (const item of assistance.catalog) completions.set(item.label, item);
  function problems(files: YamlFiles): YamlProblem[] {
    const resultProblems: YamlProblem[] = [];
    for (const [file, text] of Object.entries(files)) {
      const counter = new LineCounter();
      const document = parseDocument(text, {
        customTags: yamlTags,
        lineCounter: counter,
      });
      if (document.errors.length) continue;
      const validate = validators.get(
        file === 'settings.yml' ? 'settings-schema.json' : 'config-schema.json',
      );
      if (!validate || file.startsWith('includes/')) continue;
      let result;
      try {
        result = validate.validate(document.toJS({ maxAliasCount: 100 }));
      } catch {
        continue;
      }
      for (const error of result.errors.slice(0, 100)) {
        const path = error.instanceLocation
          .replace(/^#/, '')
          .split('/')
          .slice(1)
          .map((part) => part.replaceAll('~1', '/').replaceAll('~0', '~'));
        const node = document.getIn(path, true);
        const position = counter.linePos(
          isNode(node) ? (node.range?.[0] ?? 0) : 0,
        );
        resultProblems.push({
          file,
          line: position.line,
          column: position.col,
          message: `${path.join('.') || 'Document'}: ${error.error}`,
        });
      }
    }
    return resultProblems;
  }
  return { completions: [...completions.values()], documentation, problems };
}
