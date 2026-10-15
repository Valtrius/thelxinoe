import { expect, test, type Page, type TestInfo } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

// Browser-only coverage of the two service stages and the guided transfer modal.
const steps = [
  ['prerequisites', 'Check prerequisites', 'backend'],
  ['review', 'Review changes', 'user'],
  ['automation', 'Confirm automation is retired', 'user'],
  ['confirm', 'Confirm the transfer', 'user'],
  ['stop', 'Stop the original', 'backend'],
  ['copy', 'Copy appdata', 'backend'],
  ['validate', 'Validate a disposable copy', 'backend'],
  ['activate', 'Start the managed replacement', 'backend'],
  ['commit', 'Commit ownership', 'backend'],
  ['setup', 'Run reviewed setup', 'backend'],
  ['checks', 'Verify required checks', 'backend'],
  ['complete', 'Complete the transfer', 'backend'],
] as const;
const checks = [
  ['docker_storage', 'Accepted Docker and storage identity', true],
  ['preserved_state', 'Preserved application state', true],
  ['api_ready', 'Authenticated API readiness', true],
  ['links', 'Radarr application link', false],
].map(([id, label, required]) => ({
  id,
  label,
  required,
  status: 'pending',
  detail: null,
  evidence: null,
}));

function view(
  state: string,
  step: string,
  extra: Record<string, unknown> = {},
) {
  const at = steps.findIndex(([key]) => key === step);
  return {
    id: 'transfer-prowlarr',
    kind: 'prowlarr',
    service_id: 'support-prowlarr',
    state,
    step,
    steps: steps.map(([key, label, actor], index) => ({
      key,
      label,
      actor,
      status:
        state === 'complete'
          ? 'complete'
          : state === 'review'
            ? index === 0
              ? 'complete'
              : index < 4
                ? 'awaiting_input'
                : 'pending'
            : index < at
              ? 'complete'
              : index === at
                ? state === 'failed'
                  ? 'failed'
                  : 'running'
                : 'pending',
    })),
    review: {
      plan: {
        review_id: 'transfer-prowlarr',
        revision: 'plan-revision',
        kind: 'prowlarr',
        source: {
          container_id: 'container-prowlarr'.padEnd(64, '0'),
          name: 'prowlarr',
          image: 'lscr.io/linuxserver/prowlarr@sha256:source',
          version: '2.6.5.5623-ls162',
          running: true,
          restart_policy: 'unless-stopped',
          networks: ['media'],
          appdata: {
            kind: 'bind',
            source: '/srv/appdata/prowlarr',
            destination: '/config',
            writable: true,
            role: 'appdata',
          },
          identity: { uid: 1000, gid: 1000 },
        },
        target: {
          image: 'lscr.io/linuxserver/prowlarr@sha256:source',
          version: '2.6.5.5623-ls162',
          decision: { decision: 'same', image: 'same' },
          appdata:
            'Thelxinoe deployment storage (services/transfer-prowlarr/appdata)',
          appdata_destination: '/config',
          identity: { uid: 10001, gid: 10001 },
          aliases: ['prowlarr'],
          published_port: null,
          mounts: [],
          supplementary: [],
        },
        changes: [
          {
            area: 'ports',
            before: '0.0.0.0:9696 → 9696/tcp',
            after: 'Not published',
            reason:
              'Managed services publish only on this host’s loopback address',
          },
        ],
        settings: [
          {
            setting: 'URL Base',
            before: '',
            after: '/services/prowlarr',
            reason: 'Thelxinoe opens the service at its managed route',
          },
        ],
        automation: [
          {
            kind: 'docker_access',
            name: 'portainer',
            scope: 'Unknown: this container controls Docker',
            blocks: false,
          },
        ],
        compose: {
          project: 'media',
          service: 'prowlarr',
          working_dir: '/srv/compose',
          config_files: '/srv/compose/compose.yaml',
          unrelated: [{ name: 'jellyfin', service: 'jellyfin', running: true }],
        },
        blockers: [],
      },
      blockers: [],
      links: [
        {
          kind: 'indexer_sync',
          peer: 'Radarr',
          peer_kind: 'radarr',
          direction: 'outgoing',
          action: 'connect_after_completion',
        },
      ],
      dependencies: [],
      checks,
      setup_actions: [
        { action: 'prowlarr.hosts', label: 'Register managed host aliases' },
      ],
      recyclarr_targets: [],
    },
    review_revision: 'review-1',
    choices: {},
    checks: [],
    setup: [],
    recovery: {},
    progress: null,
    error: null,
    replacement_id: null,
    actions: {
      approve: state === 'review',
      cancel: state === 'review' || state === 'queued',
      retry: false,
      restore: false,
      recovery: false,
      remove_original: state === 'complete',
      continue_in_background: ['queued', 'running', 'committed'].includes(
        state,
      ),
    },
    controller:
      state === 'review'
        ? null
        : {
            stage: state,
            boundary_at: at >= 7 ? 1789984800 : null,
            completed: state === 'complete',
            source: {
              stopped: at >= 4,
              restart_disabled: at >= 4,
              removed: false,
            },
          },
    ...extra,
  };
}

async function capture(page: Page, testInfo: TestInfo, name: string) {
  for (const [width, height] of [
    [1440, 1000],
    [390, 844],
  ]) {
    await page.setViewportSize({ width, height });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    const path = testInfo.outputPath(`${name}-${width}.png`);
    await page.screenshot({ path });
    await testInfo.attach(`${name}-${width}`, {
      path,
      contentType: 'image/png',
    });
  }
  await page.setViewportSize({ width: 1440, height: 1000 });
}

test('a Connected service is validation-only and its transfer is guided step by step', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
    connected: ['prowlarr'],
  });
  let current = view('review', 'review');
  const approvals: unknown[] = [];
  const posts: string[] = [];
  await page.route('**/api/v1/admin/services/support-prowlarr/*', (route) => {
    const action = new URL(route.request().url()).pathname.split('/').pop()!;
    posts.push(action);
    return route.fulfill({
      json: action === 'test' ? { healthy: true, version: '2.0.0' } : current,
    });
  });
  await page.route('**/api/v1/admin/transfers/transfer-prowlarr**', (route) => {
    const path = new URL(route.request().url()).pathname;
    if (route.request().method() !== 'GET')
      posts.push(path.split('/transfer-prowlarr/')[1]);
    if (path.endsWith('/choices')) {
      current = {
        ...current,
        choices: route.request().postDataJSON(),
        review_revision: 'review-2',
      };
    } else if (path.endsWith('/approve')) {
      approvals.push(route.request().postDataJSON());
      current = view('running', 'copy', {
        progress: { phase: 'copying', bytes: 48 * 1024 ** 2, entries: 310 },
      });
      fixture.operations.set('prowlarr', {
        type: 'transfer',
        id: 'transfer-prowlarr',
        state: 'running',
        step: 'copy',
        error: null,
      });
    }
    return route.fulfill({ json: current });
  });
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('link', { name: 'Prowlarr', exact: true })
    .click();
  const connected = page.getByRole('region', { name: 'Connected service' });
  await expect(connected).toContainText('Verified');
  await expect(connected).toContainText('fixture-engine');
  // Operational controls stay unavailable until Thelxinoe owns the service.
  await expect(
    page.getByRole('switch', { name: 'Enable Fixture indexer' }),
  ).toHaveCount(0);
  await page.getByRole('button', { name: 'Test connection' }).click();
  await expect.poll(() => posts).toEqual(['test']);
  await capture(page, testInfo, 'connected-stage');

  await page
    .getByRole('button', { name: 'Take ownership', exact: true })
    .click();
  const dialog = page.getByRole('dialog', {
    name: 'Take ownership of Prowlarr',
  });
  const nav = dialog.getByRole('navigation', {
    name: 'Ownership transfer steps',
  });
  await expect(nav.getByRole('button')).toHaveCount(12);
  await expect(
    nav.getByRole('button', {
      name: '2. Review changes: Your input, your action',
    }),
  ).toHaveAttribute('aria-current', 'step');
  await expect(
    dialog.getByRole('region', { name: 'Deployment changes' }),
  ).toContainText('0.0.0.0:9696 → 9696/tcp');
  await expect(
    dialog.getByRole('region', { name: 'Application changes' }),
  ).toContainText('/services/prowlarr');
  await expect(
    dialog.getByRole('region', { name: 'Service links' }),
  ).toContainText('connects automatically after completion');
  await capture(page, testInfo, 'transfer-review');
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();

  const automation = dialog.getByRole('region', {
    name: 'Confirm automation is retired',
  });
  await expect(automation).toContainText('portainer');
  const next = dialog.getByRole('button', { name: 'Continue', exact: true });
  await expect(next).toBeDisabled();
  await dialog
    .getByRole('button', { name: "Retire the whole project's automation" })
    .click();
  for (const name of [
    'No off-host or host cron job changes the original or its copy',
    'No systemd unit or timer restarts or recreates it',
    'No CI/CD pipeline deploys it',
    'No configuration-management tool (Ansible, Salt, Puppet, Chef) manages it',
    'No one runs manual Compose commands that start or recreate it',
    "portainer won't start, recreate or remove the original",
    'I accept losing automated control of those services',
    'No automation or manual command will run against the project',
  ])
    await dialog.getByRole('switch', { name }).check();
  await capture(page, testInfo, 'transfer-automation');
  await next.click();

  const take = dialog.getByRole('button', {
    name: 'Take ownership',
    exact: true,
  });
  await expect(take).toBeDisabled();
  await dialog
    .getByRole('switch', {
      name: 'I reviewed every change, and I understand the original stops',
    })
    .check();
  await capture(page, testInfo, 'transfer-confirm');
  await take.click();
  await expect.poll(() => approvals.length).toBe(1);
  expect(approvals[0]).toMatchObject({
    review_revision: 'review-2',
    confirm: true,
    declarations: {
      cron: true,
      systemd: true,
      ci_cd: true,
      configuration_management: true,
      manual_compose: true,
      unobservable_automation: true,
      project_retired: true,
      unrelated_services_acknowledged: true,
    },
  });

  // Backend steps show only reported progress; closing never cancels them.
  await expect(
    nav.getByRole('button', {
      name: '6. Copy appdata: In progress, done by Thelxinoe',
    }),
  ).toBeVisible();
  await expect(dialog).toContainText('48.0 MiB processed');
  await expect(
    dialog.getByRole('button', { name: 'Continue in background' }).first(),
  ).toBeVisible();
  await capture(page, testInfo, 'transfer-progress');
  // Escape only dismisses the window; the operation keeps running and reopens as-is.
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await page
    .getByRole('button', { name: 'Open ownership transfer', exact: true })
    .click();
  await expect(
    nav.getByRole('button', {
      name: '6. Copy appdata: In progress, done by Thelxinoe',
    }),
  ).toBeVisible();
  expect(posts.filter((action) => action === 'transfer')).toHaveLength(1);
  expect(approvals).toHaveLength(1);
  expect(posts).not.toContain('cancel');
  // Steps are reachable from the keyboard.
  await nav.getByRole('button', { name: /^2. Review changes/ }).focus();
  await page.keyboard.press('Enter');
  await expect(
    nav.getByRole('button', { name: /^2. Review changes/ }),
  ).toHaveAttribute('aria-current', 'step');
  fixture.operations.delete('prowlarr');

  current = view('complete', 'complete', {
    checks: checks.map((check) => ({
      ...check,
      status: check.required ? 'passed' : 'pending',
      detail: check.required
        ? null
        : 'Connects automatically after completion when both services are ready',
    })),
  });
  await expect(dialog).toContainText('Prowlarr is now Owned by Thelxinoe.', {
    timeout: 10000,
  });
  await expect(dialog).toContainText('Unavailable');
  await capture(page, testInfo, 'transfer-complete');
  await dialog
    .getByRole('button', { name: 'Remove the stopped original container' })
    .click();
  await expect.poll(() => posts).toContain('original/remove');
  await dialog
    .getByRole('button', { name: 'Open Prowlarr', exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page
      .getByRole('navigation', { name: 'Select service' })
      .getByRole('link', { name: 'Prowlarr', exact: true }),
  ).toHaveAttribute('aria-current', 'true');
  await capture(page, testInfo, 'completed-transfer-service-opened');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('a failure after production offers evidence-based recovery choices', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  const failed = view('failed', 'checks', {
    error: 'Required check failed: Authenticated API readiness',
    actions: {
      approve: false,
      cancel: false,
      retry: true,
      restore: false,
      recovery: true,
      remove_original: false,
      continue_in_background: false,
    },
  });
  // The approved command runs in the controller; the modal follows it until it ends.
  const recovering = view('recovery', 'checks', {
    recovery: {
      command: { operation: 'operation-1', action: 'return_original' },
    },
    controller: {
      stage: 'recovering',
      boundary_at: 1789984800,
      completed: false,
      source: { stopped: true, restart_disabled: true, removed: false },
      command: {
        operation: 'operation-1',
        phase: 'starting-original',
        state: 'running',
      },
    },
    actions: { ...failed.actions, retry: false, recovery: false },
  });
  let current = failed;
  const applied: unknown[] = [];
  await page.route('**/api/v1/admin/services', (route) =>
    route.fulfill({
      json: {
        items: [
          {
            kind: 'prowlarr',
            stage: 'owned',
            hold: 'transfer',
            agreed: true,
            draft_review: null,
            actions: [],
            connection: null,
            ownership: {
              id: 'transfer-prowlarr',
              state: 'complete',
              origin: 'transferred',
              hold: 'transfer',
              transfer_id: 'transfer-prowlarr',
            },
            operation: {
              type: 'transfer',
              id: 'transfer-prowlarr',
              state: 'failed',
              step: 'checks',
              error: failed.error,
            },
          },
        ],
      },
    }),
  );
  await page.route('**/api/v1/admin/transfers/transfer-prowlarr**', (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith('/recovery') && route.request().method() === 'GET')
      return route.fulfill({
        json: {
          revision: 'recovery-1',
          evidence: {
            boundary_at: 1789984800,
            images: { source: 'source', replacement: 'replacement' },
            versions: { source: '2.6.5', replacement: '2.6.5' },
            containers: {
              replacement: { present: true, running: true },
              original: { present: true, running: false, unchanged: true },
            },
            effects: {
              known: [{ effect: 'indexer changes', count: 2 }],
              uncertain: ['Downloads grabbed after activation are not tracked'],
            },
          },
          options: [
            {
              action: 'return_original',
              label: 'Return to the original',
              available: true,
              description: 'Start the stopped original and remove the copy',
              requires: ['state_loss', 'reconciliation'],
            },
            {
              action: 'repair',
              label: 'Repair and continue',
              available: false,
              unavailable:
                'The original is running; a replacement never starts beside it',
              description: 'Start the replacement again',
            },
            {
              action: 'suspend',
              label: 'Suspend for manual resolution',
              available: true,
              description: 'Keep everything held and stopped',
            },
          ],
          cannot_undo: ['Indexer searches already sent'],
          transfer_written: [],
          peer_note: '',
        },
      });
    if (path.endsWith('/recovery')) {
      applied.push(route.request().postDataJSON());
      current = recovering;
    }
    return route.fulfill({ json: current });
  });
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('link', { name: 'Prowlarr', exact: true })
    .click();
  await expect(
    page.getByText('Owned, under an operational hold'),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Open transfer recovery' }).click();
  const dialog = page.getByRole('dialog', {
    name: 'Take ownership of Prowlarr',
  });
  await expect(dialog.getByRole('alert')).toContainText(
    'The replacement reached production',
  );
  await dialog.getByRole('button', { name: 'Review recovery options' }).click();
  const plan = dialog.getByRole('region', { name: 'Recovery plan' });
  await expect(plan).toContainText('Stopped, unchanged');
  await expect(plan).toContainText('Indexer searches already sent');
  // A replacement never starts beside a running original; the plan says why.
  await expect(plan).toContainText(
    'Unavailable: The original is running; a replacement never starts beside it',
  );
  await expect(
    plan.getByRole('radio', { name: /Repair and continue/ }),
  ).toBeDisabled();
  await plan.getByRole('radio', { name: /Return to the original/ }).check();
  const apply = plan.getByRole('button', { name: 'Apply recovery' });
  await expect(apply).toBeDisabled();
  await plan
    .getByRole('switch', {
      name: 'I accept that application state changed since activation may be lost',
    })
    .check();
  await plan
    .getByRole('switch', {
      name: 'I will reconcile peers, downloads and media listed above myself',
    })
    .check();
  await capture(page, testInfo, 'transfer-recovery');
  await apply.click();
  await expect
    .poll(() => applied)
    .toEqual([
      {
        action: 'return_original',
        revision: 'recovery-1',
        approval: { state_loss: true, reconciliation: true },
      },
    ]);
  await expect(dialog.getByRole('status').first()).toContainText(
    'Applying the approved recovery: Return to the original',
  );
  await expect(dialog).toContainText('Restarting the original');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
