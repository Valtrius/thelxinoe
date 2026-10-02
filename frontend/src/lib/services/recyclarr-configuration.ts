import { diff } from '@codemirror/merge';

export type YamlFiles = Record<string, string>;
export type YamlProblem = {
  file: string;
  line: number;
  column: number;
  message: string;
};
export type RecyclarrBundle = {
  image: string;
  resources: string;
  files: YamlFiles;
};
export type RecyclarrConfiguration = {
  revision: string;
  mode: 'defaults' | 'customized';
  image: string;
  files: YamlFiles;
  defaults: RecyclarrBundle;
  base_defaults: RecyclarrBundle;
  bindings: {
    service_id: string;
    kind: string;
    instance: string;
    base_url: string;
    api_key: string;
  }[];
  candidate: null | {
    operation_id: string;
    base_revision: string;
    image: string;
    files: YamlFiles;
    defaults: RecyclarrBundle;
    valid: boolean;
    edited: boolean;
    diagnostics: YamlProblem[];
  };
};
export function sameFiles(a: YamlFiles, b: YamlFiles) {
  const names = Object.keys(a);
  return (
    names.length === Object.keys(b).length &&
    names.every((name) => a[name] === b[name])
  );
}
export function yamlFilePath(path: string) {
  return (
    path.length <= 200 &&
    /^(recyclarr\.yml|settings\.yml|configs\/[\w.-]+\.ya?ml|includes\/(?:[\w.-]+\/)*[\w.-]+\.ya?ml)$/.test(
      path,
    ) &&
    path.split('/').every((part) => part !== '.' && part !== '..')
  );
}

export async function mergeDefaults(
  base: YamlFiles,
  custom: YamlFiles,
  incoming: YamlFiles,
) {
  const files = { ...custom };
  const conflicts: string[] = [];
  for (const file of new Set([
    ...Object.keys(base),
    ...Object.keys(custom),
    ...Object.keys(incoming),
  ])) {
    const previous = base[file],
      current = custom[file],
      next = incoming[file];
    if (current === next || next === previous) continue;
    if (current === previous) {
      if (next === undefined) delete files[file];
      else files[file] = next;
      continue;
    }
    if (previous === undefined || current === undefined || next === undefined) {
      conflicts.push(file);
      continue;
    }
    const edits = diff(previous, current),
      updates = diff(previous, next);
    const applicable = updates.filter(
      (update) =>
        !edits.some(
          (edit) => update.fromA <= edit.toA && edit.fromA <= update.toA,
        ),
    );
    if (applicable.length !== updates.length) conflicts.push(file);
    for (const update of [...applicable].reverse()) {
      const offset = edits
        .filter((edit) => edit.toA < update.fromA)
        .reduce(
          (sum, edit) =>
            sum + (edit.toB - edit.fromB) - (edit.toA - edit.fromA),
          0,
        );
      files[file] =
        files[file].slice(0, update.fromA + offset) +
        next.slice(update.fromB, update.toB) +
        files[file].slice(update.toA + offset);
    }
  }
  return { files, conflicts };
}
