import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { jsonWrite } from './ci-state.mjs';
import { cleanupResources, resourceScope } from './ci-resources.mjs';
import { imageManifestHash, imageManifest } from './ci-images.mjs';

function version(command, args) {
  const result = spawnSync(command, args, {
    encoding: 'utf8',
    windowsHide: true,
    timeout: 30000,
  });
  return result.status === 0 ? result.stdout.trim() : null;
}
export function qualification(phases) {
  resourceScope();
  mkdirSync('.local', { recursive: true });
  const require = createRequire(import.meta.url);
  let browserVersion = null;
  try {
    browserVersion = require('@playwright/test/package.json').version;
  } catch {
    /* The Linux verification image does not need browser dependencies. */
  }
  const report = {
    run_id: process.env.THELXINOE_CI_RUN_ID,
    revision:
      process.env.THELXINOE_CI_REVISION ??
      version('git', ['rev-parse', 'HEAD']),
    source_sha256: process.env.THELXINOE_CI_SOURCE_SHA256 ?? null,
    image_manifest_sha256: imageManifestHash,
    image_inputs: imageManifest,
    evidence: {
      steps: '.local/ci-steps.json',
      commands: '.local/ci-command.log',
      staging: '.local/ci-image-staging.json',
      product: 'test-results/',
    },
    environment: {
      platform: process.platform,
      arch: process.arch,
      node: process.version,
      pnpm: process.env.THELXINOE_CI_IN_VERIFY
        ? null
        : version(
            process.platform === 'win32'
              ? (process.env.ComSpec ?? 'cmd.exe')
              : 'pnpm',
            process.platform === 'win32'
              ? ['/d', '/s', '/c', 'pnpm --version']
              : ['--version'],
          ),
      rust:
        process.env.THELXINOE_CI_IN_VERIFY ||
        phases.some((phase) => ['desktop', 'updates-desktop'].includes(phase))
          ? version('rustc', ['--version'])
          : null,
      docker:
        phases.some((phase) =>
          ['server', 'containers', 'updates-server'].includes(phase),
        ) && !process.env.THELXINOE_CI_IN_VERIFY
          ? version('docker', ['version', '--format', '{{json .}}'])
          : null,
      playwright: browserVersion,
    },
    started: new Date().toISOString(),
    finished: null,
    passed: null,
    phases: [],
    cleanup_errors: [],
  };
  const save = () => jsonWrite('.local/ci-result.json', report);
  save();
  let active;
  return {
    report,
    save,
    begin(phase) {
      active = {
        phase,
        started: new Date().toISOString(),
        finished: null,
        passed: null,
        groups: [],
        browser: {
          transport:
            phase === 'web'
              ? 'host'
              : phase === 'updates-desktop'
                ? 'host and Windows native CDP'
                : phase === 'desktop'
                  ? 'Windows native'
                  : 'none',
        },
      };
      report.phases.push(active);
      save();
    },
    async group(name, action, requires = [], productEvidence) {
      const group = {
        name,
        started: new Date().toISOString(),
        finished: null,
        state: 'running',
        product_passed: null,
        cleanup_errors: [],
        product_evidence: productEvidence ?? null,
      };
      active.groups.push(group);
      const missing = requires.filter(
        (required) =>
          !active.groups.some(
            (item) => item.name === required && item.state === 'passed',
          ),
      );
      if (missing.length) {
        group.state = 'skipped';
        group.reason = `Failed prerequisites: ${missing.join(', ')}`;
        group.finished = new Date().toISOString();
        group.elapsed_ms =
          Date.parse(group.finished) - Date.parse(group.started);
        save();
        return;
      }
      const original = process.env.THELXINOE_CI_RESOURCE_DIRECTORY;
      const resources = join(original, active.phase, name);
      process.env.THELXINOE_CI_RESOURCE_DIRECTORY = resources;
      process.env.THELXINOE_CI_GROUP = name;
      save();
      try {
        await action();
        group.product_passed = true;
        group.state = 'passed';
      } catch (error) {
        group.error = error.message;
        group.product_passed = false;
        group.state = 'failed';
        console.error(`${name}: ${error.message}`);
      } finally {
        if (productEvidence) {
          try {
            if (
              statSync(productEvidence).mtimeMs >= Date.parse(group.started)
            ) {
              const outcome = JSON.parse(readFileSync(productEvidence, 'utf8'));
              const passed = outcome.product_passed ?? outcome.passed;
              if (typeof passed === 'boolean') {
                group.product_passed = passed;
                if (!passed) {
                  group.state = 'failed';
                  group.error = outcome.error ?? group.error;
                } else if (group.state === 'failed')
                  group.finalization_error = group.error;
              }
              group.cleanup_errors.push(...(outcome.cleanup_errors ?? []));
            }
          } catch {
            /* Command and resource evidence still record early setup failures. */
          }
        }
        try {
          group.cleanup_errors.push(
            ...cleanupResources(resources, report.run_id),
          );
        } catch (error) {
          group.cleanup_errors.push(error.message);
        }
        if (group.cleanup_errors.length) group.state = 'failed';
        process.env.THELXINOE_CI_RESOURCE_DIRECTORY = original;
        delete process.env.THELXINOE_CI_GROUP;
        group.finished = new Date().toISOString();
        group.elapsed_ms =
          Date.parse(group.finished) - Date.parse(group.started);
        save();
      }
    },
    end(error) {
      if (error) active.error = error.message;
      active.passed =
        !error && active.groups.every((group) => group.state === 'passed');
      active.finished = new Date().toISOString();
      active.elapsed_ms =
        Date.parse(active.finished) - Date.parse(active.started);
      save();
      return active.passed;
    },
    finish() {
      try {
        report.cleanup_errors = cleanupResources(
          process.env.THELXINOE_CI_RESOURCE_DIRECTORY,
          report.run_id,
          { removeImages: true },
        );
      } catch (error) {
        report.cleanup_errors.push(error.message);
      }
      report.finished = new Date().toISOString();
      report.passed =
        report.phases.every((phase) => phase.passed) &&
        !report.cleanup_errors.length;
      save();
      return report.passed;
    },
  };
}
