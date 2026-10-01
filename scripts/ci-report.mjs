import { readFileSync, existsSync } from 'node:fs';
import { basename, join } from 'node:path';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { atomicWrite, originLabel } from './ci-state.mjs';

const escape = (value) =>
  String(value ?? '').replace(
    /[&<>"']/g,
    (character) =>
      ({
        '&': '&amp;',
        '<': '&lt;',
        '>': '&gt;',
        '"': '&quot;',
        "'": '&#39;',
      })[character],
  );
const duration = (lane) =>
  lane.started
    ? Math.round(
        ((lane.finished ? Date.parse(lane.finished) : Date.now()) -
          Date.parse(lane.started)) /
          1000,
      ) + ' s'
    : '—';

function stepDetails(lane) {
  const steps = lane.steps ?? [];
  const current = steps.at(-1);
  if (!current) return '';
  return `<p class="mb-2 max-w-lg break-all">${current.finished ? 'Last command' : 'Running'}: <code>${escape(current.command)}</code> · ${duration(current)}</p><details class="mb-3"><summary class="cursor-pointer">${steps.length} commands</summary><ol class="mt-2 space-y-2">${steps.map((step) => `<li class="max-w-lg break-all">${step.finished ? (step.passed ? 'PASS' : 'FAIL') : 'RUNNING'} · ${duration(step)} · <code>${escape(step.command)}</code></li>`).join('')}</ol></details>`;
}

function page(report, css) {
  const state = report.finished
    ? report.passed
      ? 'Passed'
      : 'Failed'
    : 'Running';
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">${report.finished ? '' : '<meta http-equiv="refresh" content="5">'}<title>${escape(originLabel(report.origin))} - ${escape(basename(report.origin?.worktree ?? report.directory ?? ''))} - ${state} - CI ${escape(report.id)}</title><style>${css}</style></head><body class="bg-slate-50 text-slate-900 dark:bg-slate-950 dark:text-slate-100 print:bg-white print:text-slate-900 p-4 sm:p-8"><main class="mx-auto max-w-7xl space-y-8"><header class="space-y-3"><p class="text-sm text-slate-500 dark:text-slate-400">Local CI · ${escape(report.id)}</p><h1 class="text-3xl font-semibold break-all">${escape(originLabel(report.origin))}</h1><p class="break-all font-mono text-sm">${escape(report.origin?.worktree ?? 'Unknown originating worktree')}</p><p class="text-xl font-semibold">${state} · ${report.origin?.dirty ? 'Working-tree changes' : 'Clean snapshot'}</p><p>Started ${escape(report.started)}${report.finished ? ` · Finished ${escape(report.finished)}` : ' · Refreshes every five seconds'}</p><p class="break-all text-sm">Source ${escape(report.source?.revision ?? 'Capturing snapshot')} · Snapshot ${escape(report.source?.sha256 ?? 'pending')}</p><p>Each lane has its own workspace, dependencies, build outputs and evidence.</p></header>${report.cleanup_errors?.length ? `<p class="text-red-700 dark:text-red-300">Cleanup failed: ${escape(report.cleanup_errors.join('; '))}</p>` : ''}${report.error ? `<p class="text-red-700 dark:text-red-300">${escape(report.error)}</p>` : ''}<section aria-labelledby="lanes-title" class="space-y-3"><h2 id="lanes-title" class="text-xl font-semibold">Lanes</h2><div class="overflow-x-auto"><table class="w-full min-w-[48rem] text-left text-sm"><caption class="sr-only">Local CI lane status and evidence</caption><thead><tr class="border-b border-slate-300 dark:border-slate-700"><th scope="col" class="p-3">Lane</th><th scope="col" class="p-3">Status</th><th scope="col" class="p-3">Elapsed</th><th scope="col" class="p-3">Results</th></tr></thead><tbody>${report.lanes.map((lane) => `<tr class="border-b border-slate-200 dark:border-slate-800"><th scope="row" class="p-3 font-medium">${escape(lane.phase)}</th><td class="p-3 ${lane.state === 'passed' ? 'text-emerald-700 dark:text-emerald-300' : lane.state === 'failed' ? 'text-red-700 dark:text-red-300' : ''}">${escape(lane.state)}${lane.error ? `<p class="mt-1 max-w-lg break-words">${escape(lane.error)}</p>` : ''}</td><td class="p-3 whitespace-nowrap">${duration(lane)}</td><td class="p-3">${stepDetails(lane)}<a class="underline text-blue-700 dark:text-blue-300" href="phases/${escape(lane.phase)}/output.log">Log</a> · <a class="underline text-blue-700 dark:text-blue-300" href="${escape(lane.evidence ?? `workspaces/${lane.phase}`)}/">Evidence</a>${lane.phase === 'web' ? ` · <a class="underline text-blue-700 dark:text-blue-300" href="${escape(lane.evidence ?? 'workspaces/web')}/playwright-report/layout/index.html">UI report</a>` : ''}</td></tr>`).join('')}</tbody></table></div></section><section class="space-y-3" aria-labelledby="source-title"><h2 id="source-title" class="text-xl font-semibold">Reproduce and inspect</h2><p class="break-all"><a href="result.json" class="underline text-blue-700 dark:text-blue-300">Machine-readable result</a> · <a href="source.json" class="underline text-blue-700 dark:text-blue-300">Snapshot identity</a></p><p>${report.cleaned ? 'Lane worktrees were removed; source/ retains the tested snapshot and evidence/ retains qualification artifacts.' : 'Each workspace retains the tested source, test reports and qualification artifacts. To rerun a lane, open its workspace and run'} <code>node scripts/ci.mjs &lt;lane&gt;</code>.</p></section></main></body></html>`;
}

export function reporter(directory, report) {
  let css = existsSync(join(directory, 'report.css'))
    ? readFileSync(join(directory, 'report.css'), 'utf8')
    : '';
  let styling;
  function save() {
    for (const lane of report.lanes) {
      const path =
        lane.workspace && join(lane.workspace, '.local/ci-steps.json');
      if (path && existsSync(path)) {
        try {
          lane.steps = JSON.parse(readFileSync(path, 'utf8'));
        } catch {
          // A subprocess may be writing its next step; retry on the next refresh.
        }
      }
    }
    atomicWrite(
      join(directory, 'result.json'),
      JSON.stringify(report, null, 2) + '\n',
    );
    atomicWrite(
      join(directory, 'summary.txt'),
      [
        `Thelxinoe CI ${report.id}: ${report.finished ? (report.passed ? 'PASS' : 'FAIL') : 'RUNNING'}`,
        `Branch: ${originLabel(report.origin)}`,
        `Worktree: ${report.origin?.worktree ?? 'Unknown originating worktree'}`,
        `Source: ${report.origin?.revision ?? report.source?.revision ?? 'pending'} (${report.origin?.dirty ? 'working-tree changes' : 'clean'})`,
        ...report.lanes.map(
          (lane) =>
            `${lane.phase.padEnd(18)} ${lane.state.padEnd(10)} ${duration(lane)}${lane.error ? ' ' + lane.error : ''}`,
        ),
        report.error ?? '',
      ]
        .filter(Boolean)
        .join('\n') + '\n',
    );
    atomicWrite(join(directory, 'index.html'), page(report, css));
  }
  async function style(workspace) {
    if (styling) return styling;
    styling = (async () => {
      const require = createRequire(join(workspace, 'package.json'));
      const module = await import(
        pathToFileURL(require.resolve('@tailwindcss/node')).href
      );
      const compiler = await module.compile('@import "tailwindcss";', {
        base: workspace,
        onDependency() {},
      });
      const candidates = [
        ...page(report, '').matchAll(/class="([^"]+)"/g),
      ].flatMap((match) => match[1].split(/\s+/));
      css = compiler.build([
        ...new Set([
          ...candidates,
          'text-emerald-700',
          'dark:text-emerald-300',
          'text-red-700',
          'dark:text-red-300',
          'mt-1',
          'max-w-lg',
          'break-words',
          'mb-2',
          'mb-3',
          'cursor-pointer',
          'mt-2',
          'space-y-2',
        ]),
      ]);
      atomicWrite(join(directory, 'report.css'), css);
      save();
    })().catch((error) => {
      styling = undefined;
      throw error;
    });
    return styling;
  }
  return { save, style };
}
