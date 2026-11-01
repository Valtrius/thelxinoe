import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { statusLabel } from './service-view.mjs';

const cases = JSON.parse(
  await readFile(
    new URL(
      '../../apps/server/src/managers/status_cases.json',
      import.meta.url,
    ),
    'utf8',
  ),
);

test("the mocks label services by the server's status cases", () => {
  for (const { service, unavailable = false, status } of cases)
    assert.deepEqual(
      statusLabel(service, unavailable),
      status,
      JSON.stringify(service),
    );
});
