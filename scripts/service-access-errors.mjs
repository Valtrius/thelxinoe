import { mkdirSync, writeFileSync } from 'node:fs';

// Retain the assertion even when the standalone E2E exits before tracing stops.
process.on('uncaughtExceptionMonitor', (error) => {
  mkdirSync('test-results/service-access', { recursive: true });
  writeFileSync(
    'test-results/service-access/failure.txt',
    error.stack || String(error),
  );
});
