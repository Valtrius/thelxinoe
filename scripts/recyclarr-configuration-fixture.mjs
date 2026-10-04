import { expect } from '@playwright/test';
import { createRequire } from 'node:module';
const { parseDocument } = createRequire(
  new URL('../frontend/package.json', import.meta.url),
)('yaml');

export async function exerciseConfiguration({
  api,
  request,
  waitRun,
  record,
  evidence,
  retryInterruptedRun,
  readRunFiles,
  withOffline,
}) {
  const original = await api('/admin/recyclarr/configuration');
  expect(original.mode).toBe('defaults');
  expect(original.files).toEqual(original.defaults.files);
  expect(original.files['recyclarr.yml']).toContain('!secret');
  const files = {
    ...original.files,
    'recyclarr.yml': `# Preserved user comment\n${original.files['recyclarr.yml']}`,
    'includes/local.yml':
      '# A reusable include\ndelete_old_custom_formats: false\n',
  };
  // Exercise resolution of an actual local include, not merely an unused file.
  files['recyclarr.yml'] = files['recyclarr.yml'].replace(
    / {4}delete_old_custom_formats: false/g,
    '    include:\n      - config: local.yml',
  );
  const invalid = await api('/admin/recyclarr/configuration/validate', 'POST', {
    revision: original.revision,
    files: { ...files, 'includes/local.yml': 'broken: [\n' },
  });
  expect(invalid.valid).toBe(false);
  expect(invalid.diagnostics.some((d) => d.file === 'includes/local.yml')).toBe(
    true,
  );
  expect((await api('/admin/recyclarr/configuration')).revision).toBe(
    original.revision,
  );
  const traversal = await request('/admin/recyclarr/configuration', 'PUT', {
    revision: original.revision,
    files: { ...files, '../secrets.yml': 'key: unsafe' },
  });
  expect(traversal.ok()).toBe(false);
  const saved = await api('/admin/recyclarr/configuration', 'PUT', {
    revision: original.revision,
    files,
  });
  expect(saved.mode).toBe('customized');
  expect(saved.files).toEqual(files);
  const stale = await request('/admin/recyclarr/configuration', 'PUT', {
    revision: original.revision,
    files: original.files,
  });
  expect(stale.status()).toBe(409);
  const preview = await api('/admin/recyclarr/preview', 'POST', {
    revision: saved.revision,
    files: {
      ...files,
      'recyclarr.yml': `# Unsaved preview\n${files['recyclarr.yml']}`,
    },
  });
  await waitRun(preview.id, 'previewed');
  const previewed = await api(`/admin/recyclarr/runs/${preview.id}`);
  expect(previewed.evidence.before).toEqual(previewed.evidence.after);
  expect((await api('/admin/recyclarr/configuration')).files).toEqual(files);
  const sync = await api('/admin/recyclarr/sync', 'POST', {
    revision: saved.revision,
  });
  await waitRun(sync.id);
  const applied = await api(`/admin/recyclarr/runs/${sync.id}`);
  expect(applied.evidence.configuration_revision).toBe(saved.revision);
  expect((await api('/admin/recyclarr/configuration')).files).toEqual(files);
  evidence.configuration = {
    revision: saved.revision,
    files: Object.keys(files),
    run: sync.id,
  };
  record(
    'Saved YAML comments and local includes survive preview and sync; stale saves, malformed includes, and traversal are rejected',
  );
  const split = parseDocument(files['recyclarr.yml'], {
    customTags: [{ tag: '!secret', resolve: (value) => value }],
  });
  const secondary = parseDocument('');
  secondary.set('sonarr', split.get('sonarr', true));
  split.delete('sonarr');
  const splitFiles = {
    ...files,
    'recyclarr.yml': split.toString(),
    'configs/retired.yml': secondary.toString(),
    'includes/obsolete.yml':
      '# Removed after interruption\ndelete_old_custom_formats: false\n',
  };
  const splitSaved = await api('/admin/recyclarr/configuration', 'PUT', {
    revision: saved.revision,
    files: splitFiles,
  });
  const interrupted = await api('/admin/recyclarr/preview', 'POST', {});
  await waitRun(interrupted.id, 'previewed');
  const renamed = {
    ...splitFiles,
    'configs/current.yml': splitFiles['configs/retired.yml'],
  };
  delete renamed['configs/retired.yml'];
  delete renamed['includes/obsolete.yml'];
  const replacement = await api('/admin/recyclarr/configuration', 'PUT', {
    revision: splitSaved.revision,
    files: renamed,
  });
  await retryInterruptedRun(interrupted.id);
  await waitRun(interrupted.id, 'previewed');
  const retried = await api(`/admin/recyclarr/runs/${interrupted.id}`);
  const actualFiles = readRunFiles(interrupted.id);
  expect(actualFiles).toEqual(renamed);
  expect(retried.evidence.configuration_revision).toBe(replacement.revision);
  expect(retried.evidence.before).toEqual(retried.evidence.after);
  const { createHash } = await import('node:crypto');
  expect(retried.evidence.config_hash).toBe(
    createHash('sha256')
      .update(
        JSON.stringify(
          Object.fromEntries(
            Object.entries(actualFiles).sort(([a], [b]) => a.localeCompare(b)),
          ),
        ),
      )
      .digest('hex'),
  );
  evidence.interrupted_configuration = {
    run: interrupted.id,
    revision: replacement.revision,
    files: Object.keys(actualFiles),
  };
  record(
    'An interrupted operation retried with the same ID removes deleted and renamed YAML, executes only its declared file set, and records the matching hash',
  );
  const restored = await api(
    '/admin/recyclarr/configuration/defaults',
    'POST',
    { revision: replacement.revision },
  );
  expect(restored.mode).toBe('defaults');
  expect(restored.files).toEqual(restored.defaults.files);
  expect(restored.files['includes/local.yml']).toBeUndefined();
  const document = parseDocument(restored.files['recyclarr.yml'], {
    customTags: [{ tag: '!secret', resolve: (value) => value }],
  });
  for (const kind of ['radarr', 'sonarr']) {
    const selected =
      kind === 'radarr'
        ? '05fbf054ac8ad0303335026cc2632f1a'
        : 'c4cadd6b35b95f62c3d47a408e53e2f7';
    for (const instance of document.get(kind, true).items) {
      const profiles = instance.value.get('quality_profiles', true);
      profiles.items = profiles.items.filter(
        (profile) => profile.get('trash_id') === selected,
      );
      profiles.add({ name: 'Any' });
      instance.value.delete('custom_format_groups');
    }
  }
  const subset = { ...restored.files, 'recyclarr.yml': document.toString() };
  const configured = await api('/admin/recyclarr/configuration', 'PUT', {
    revision: restored.revision,
    files: subset,
  });
  await waitRun(
    (
      await api('/admin/recyclarr/sync', 'POST', {
        revision: configured.revision,
      })
    ).id,
  );
  const managers = (await api('/admin/managers')).items;
  for (const manager of managers.filter((manager) =>
    ['radarr', 'sonarr'].includes(manager.kind),
  )) {
    const options = await api(`/admin/managers/${manager.id}/options`);
    expect(options.profiles.filter((profile) => profile.trash_id)).toHaveLength(
      1,
    );
    expect(
      options.profiles.find((profile) => profile.name === 'Any').trash_id,
    ).toBeNull();
  }
  const removed = parseDocument(subset['recyclarr.yml'], {
    customTags: [{ tag: '!secret', resolve: (value) => value }],
  });
  for (const instance of removed.get('radarr', true).items)
    instance.value.get('quality_profiles', true).items = instance.value
      .get('quality_profiles', true)
      .items.filter((profile) => profile.get('name') === 'Any');
  const rejected = await request('/admin/recyclarr/configuration', 'PUT', {
    revision: configured.revision,
    files: { ...subset, 'recyclarr.yml': removed.toString() },
  });
  expect(rejected.status()).toBe(409);
  expect((await api('/admin/recyclarr/configuration')).files).toEqual(subset);
  const sonarr = managers.find((manager) => manager.kind === 'sonarr');
  const options = await api(`/admin/managers/${sonarr.id}/options`);
  const native = options.profiles.find(
    (profile) => !profile.trash_id && profile.name !== 'Any',
  );
  expect(native).toBeTruthy();
  await api(`/admin/managers/${sonarr.id}/defaults`, 'PUT', {
    ...sonarr.defaults,
    quality_profile: native.id,
  });
  const radarrOnly = parseDocument(subset['recyclarr.yml'], {
    customTags: [{ tag: '!secret', resolve: (value) => value }],
  });
  radarrOnly.delete('sonarr');
  const offlineFiles = { ...subset, 'recyclarr.yml': radarrOnly.toString() };
  let offlineSaved;
  await withOffline(['sonarr'], async () => {
    const validated = await api(
      '/admin/recyclarr/configuration/validate',
      'POST',
      { revision: configured.revision, files: offlineFiles },
    );
    expect(validated.valid).toBe(true);
    offlineSaved = await api('/admin/recyclarr/configuration', 'PUT', {
      revision: configured.revision,
      files: offlineFiles,
    });
    expect(offlineSaved.files).toEqual(offlineFiles);
    const requiredOffline = await request(
      '/admin/recyclarr/configuration/validate',
      'POST',
      { revision: offlineSaved.revision, files: subset },
    );
    expect(requiredOffline.ok()).toBe(false);
    const defaultsOffline = await request(
      '/admin/recyclarr/configuration/defaults',
      'POST',
      { revision: offlineSaved.revision },
    );
    expect(defaultsOffline.ok()).toBe(false);
  });
  await api('/admin/recyclarr/configuration/defaults', 'POST', {
    revision: offlineSaved.revision,
  });
  await api(`/admin/managers/${sonarr.id}/defaults`, 'PUT', sonarr.defaults);
  record(
    'An offline unused Sonarr does not block Radarr-only validation or save; including Sonarr or restoring defaults still requires it',
  );
  record(
    'Custom YAML tracks only configured guides and native profiles, does not re-enroll the catalog, and protects the selected acquisition profile',
  );
}

export async function exerciseConfigurationEditor({ page, api, root, record }) {
  const assistance = await api('/admin/recyclarr/configuration/editor');
  expect(assistance.documents).toHaveProperty(['config-schema.json']);
  expect(assistance.documents).toHaveProperty(['settings-schema.json']);
  const open = page.getByRole('button', { name: 'Edit YAML', exact: true });
  await open.click();
  const modal = page.getByRole('dialog', {
    name: 'Recyclarr YAML',
    exact: true,
  });
  await expect(modal).toBeVisible({ timeout: 120000 });
  await modal.getByRole('tab', { name: 'settings.yml', exact: true }).click();
  await expect(modal.locator('.cm-content')).toContainText(
    'resource_providers',
  );
  await modal.getByRole('tab', { name: 'recyclarr.yml', exact: true }).click();
  await modal.getByRole('button', { name: 'Customize', exact: true }).click();
  const editor = modal.locator('.cm-content');
  await editor.click();
  await page.keyboard.press('Control+Home');
  await page.keyboard.insertText('# Editor draft\n');
  await modal.getByRole('tab', { name: 'settings.yml', exact: true }).click();
  await modal.getByRole('tab', { name: /^recyclarr.yml/ }).click();
  await expect(editor).toContainText('# Editor draft');
  await modal.getByRole('button', { name: 'Wrap lines', exact: true }).click();
  await editor.click();
  await page.keyboard.press('Control+z');
  await expect(editor).not.toContainText('# Editor draft');
  await page.keyboard.press('Control+Shift+z');
  await expect(editor).toContainText('# Editor draft');
  await page.screenshot({
    path: `${root}/yaml-editor-desktop.png`,
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect
    .poll(async () =>
      modal.evaluate((node) => node.getBoundingClientRect().right),
    )
    .toBeLessThanOrEqual(390);
  await expect(
    modal.getByRole('button', { name: 'Save', exact: true }),
  ).toBeInViewport();
  await page.screenshot({
    path: `${root}/yaml-editor-mobile.png`,
    fullPage: true,
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await modal
    .getByRole('button', { name: 'Close YAML editor', exact: true })
    .click();
  const discard = page.getByRole('dialog', {
    name: 'Discard changes?',
    exact: true,
  });
  await expect(discard).toBeVisible();
  await discard
    .getByRole('button', { name: 'Keep editing', exact: true })
    .click();
  await expect(editor).toContainText('# Editor draft');
  await modal.getByRole('button', { name: 'Save', exact: true }).click();
  await expect
    .poll(
      async () =>
        (await api('/admin/recyclarr/configuration')).files['recyclarr.yml'],
      { timeout: 180000 },
    )
    .toContain('# Editor draft');
  await modal
    .getByRole('button', { name: 'Close YAML editor', exact: true })
    .click();
  await expect(open).toBeFocused();
  const saved = await api('/admin/recyclarr/configuration');
  await api('/admin/recyclarr/configuration/defaults', 'POST', {
    revision: saved.revision,
  });
  await open.click();
  await expect(
    modal.getByRole('button', { name: 'Customize', exact: true }),
  ).toBeVisible();
  await modal
    .getByRole('button', { name: 'Close YAML editor', exact: true })
    .click();
  const schemaRoute = '**/api/v1/admin/recyclarr/configuration/editor*';
  await page.route(schemaRoute, (route) => route.abort());
  await open.click();
  await expect(
    modal.getByText(
      'Schema assistance is unavailable. Validate with Recyclarr before saving.',
      { exact: true },
    ),
  ).toBeVisible();
  await page.screenshot({
    path: `${root}/yaml-fallback-desktop.png`,
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: `${root}/yaml-fallback-mobile.png`,
    fullPage: true,
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.unroute(schemaRoute);
  await modal.getByRole('button', { name: 'Customize', exact: true }).click();
  const preview = modal.getByText('Validation preview', { exact: true });
  const validate = async () => {
    await modal
      .getByRole('button', { name: 'Validate & preview', exact: true })
      .click();
    await expect(preview).toBeVisible({ timeout: 180000 });
  };
  await validate();
  await modal
    .getByRole('button', { name: 'New YAML file', exact: true })
    .click();
  const create = page.getByRole('dialog', {
    name: 'New YAML file',
    exact: true,
  });
  await create.getByLabel('File path').fill('recyclarr.yml');
  await create
    .getByRole('button', { name: 'Create file', exact: true })
    .click();
  await expect(create.getByRole('alert')).toContainText('unique');
  await create.getByLabel('File path').fill('includes/new.yml');
  await create
    .getByRole('button', { name: 'Create file', exact: true })
    .click();
  await expect(preview).not.toBeVisible();
  await validate();
  await modal
    .getByRole('button', { name: 'Rename YAML file', exact: true })
    .click();
  const rename = page.getByRole('dialog', {
    name: 'Rename YAML file',
    exact: true,
  });
  await rename
    .getByRole('button', { name: 'Rename file', exact: true })
    .click();
  await expect(preview).toBeVisible();
  await expect(
    modal.getByRole('tab', { name: /includes\/new.yml/ }),
  ).toBeVisible();
  await modal
    .getByRole('button', { name: 'Rename YAML file', exact: true })
    .click();
  await rename.getByLabel('File path').fill('includes/renamed.yml');
  await rename
    .getByRole('button', { name: 'Rename file', exact: true })
    .click();
  await expect(preview).not.toBeVisible();
  await validate();
  await modal.locator('input[type=file]').setInputFiles({
    name: 'imported.yml',
    mimeType: 'application/yaml',
    buffer: Buffer.from('# Imported file\n{}\n'),
  });
  await expect(preview).not.toBeVisible();
  await validate();
  await modal
    .getByRole('button', { name: 'Delete YAML file', exact: true })
    .click();
  await page
    .getByRole('dialog', { name: 'Delete YAML file?', exact: true })
    .getByRole('button', { name: 'Delete file', exact: true })
    .click();
  await expect(preview).not.toBeVisible();
  await modal
    .getByRole('button', { name: 'Close YAML editor', exact: true })
    .click();
  await page
    .getByRole('dialog', { name: 'Discard changes?', exact: true })
    .getByRole('button', { name: 'Discard changes', exact: true })
    .click();
  record(
    'Tabbed editor retains drafts and undo, protects dirty close and focus, saves real configuration, fits mobile, reloads external defaults, shows schema failures, and invalidates preview for every file operation',
  );
}

export async function exerciseConfigurationUpgrades({
  api,
  request,
  withOffline,
  waitUpdate,
  services,
  snapshot,
  record,
  evidence,
  page,
  root,
  base,
}) {
  await page.goto(`${base}/?section=Settings`);
  await page.getByRole('link', { name: 'Media services', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('link', { name: 'Recyclarr', exact: true })
    .click();
  const before = Object.fromEntries(
    await Promise.all(
      ['radarr', 'sonarr'].map(async (kind) => [kind, await snapshot(kind)]),
    ),
  );
  const preflight = async (state = 'ready') => {
    const update = await api(
      `/admin/service-updates/preflight/${services.recyclarr.id}`,
      'POST',
      {},
    );
    try {
      await waitUpdate(update.id, state);
    } catch (error) {
      evidence.failed_candidate = (
        await api('/admin/recyclarr/configuration')
      ).candidate;
      throw error;
    }
    return update;
  };
  const defaults = await api('/admin/recyclarr/configuration');
  const first = await preflight();
  const reviewed = await api('/admin/recyclarr/configuration');
  expect(reviewed.files).toEqual(defaults.files);
  expect(reviewed.candidate.valid).toBe(true);
  if (page) {
    const reviewButton = page.getByRole('button', {
      name: 'Review update configuration',
      exact: true,
    });
    await expect(reviewButton).toBeVisible({ timeout: 20000 });
    await reviewButton.click();
    const modal = page.getByRole('dialog', {
      name: 'Review candidate configuration',
      exact: true,
    });
    await expect(modal).toBeVisible();
    await page.screenshot({
      path: `${root}/yaml-diff-desktop.png`,
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(modal.locator('.cm-mergeViewEditor')).toHaveCount(2);
    await page.screenshot({
      path: `${root}/yaml-diff-mobile.png`,
      fullPage: true,
    });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await modal
      .getByRole('button', { name: 'Close', exact: true })
      .last()
      .click();
  }
  await api(`/admin/service-updates/${first.id}/activate`, 'POST', {});
  await waitUpdate(first.id, 'committed');
  const upgraded = await api('/admin/recyclarr/configuration');
  expect(upgraded.mode).toBe('defaults');
  expect(upgraded.files).toEqual(reviewed.candidate.files);
  expect(upgraded.files).toEqual(upgraded.defaults.files);
  expect(upgraded.image).toBe(upgraded.defaults.image);
  expect((await api('/admin/recyclarr')).settings.paused).toBe(true);
  record(
    'A Defaults upgrade replaces the retained defaults and active file set together while automatic sync remains paused',
  );
  const customFiles = {
    ...upgraded.files,
    'recyclarr.yml': `# Custom upgrade contract\n${upgraded.files['recyclarr.yml']}`,
    'includes/retained.yml':
      '# Keep this file\ndelete_old_custom_formats: false\n',
  };
  const customized = await api('/admin/recyclarr/configuration', 'PUT', {
    revision: upgraded.revision,
    files: customFiles,
  });
  await preflight();
  const competing = await api('/admin/recyclarr/configuration');
  expect(competing.candidate.files).toEqual(customFiles);
  let candidateEditor;
  if (page) {
    await page
      .getByRole('button', { name: 'Review update configuration', exact: true })
      .click();
    await page
      .getByRole('dialog', {
        name: 'Review candidate configuration',
        exact: true,
      })
      .getByRole('button', { name: 'Edit candidate YAML', exact: true })
      .click();
    candidateEditor = page.getByRole('dialog', {
      name: 'Recyclarr YAML',
      exact: true,
    });
    await candidateEditor.locator('.cm-content').click();
    await page.keyboard.press('Control+Home');
    await page.keyboard.insertText('# Stale editor draft\n');
  }
  let edited;
  await withOffline(['radarr', 'sonarr'], async () => {
    edited = await api('/admin/recyclarr/configuration/candidate', 'POST', {
      revision: customized.revision,
      candidate_revision: competing.candidate.revision,
      files: {
        ...customFiles,
        'recyclarr.yml': `# Accepted candidate edit\n${customFiles['recyclarr.yml']}`,
        'settings.yml': 'resource_providers: invalid\n',
      },
    });
  });
  const staleCandidate = await request(
    '/admin/recyclarr/configuration/candidate',
    'POST',
    {
      revision: customized.revision,
      candidate_revision: competing.candidate.revision,
      files: customFiles,
    },
  );
  expect(staleCandidate.status()).toBe(409);
  expect((await api('/admin/recyclarr/configuration')).candidate.files).toEqual(
    edited.candidate.files,
  );
  expect(edited.revision).toBe(competing.revision);
  expect(edited.candidate.revision).not.toBe(competing.candidate.revision);
  if (candidateEditor) {
    await candidateEditor
      .getByRole('button', { name: 'Save candidate', exact: true })
      .click();
    await expect(candidateEditor.getByRole('alert')).toContainText(
      'Candidate revision changed',
    );
    await candidateEditor
      .getByRole('button', { name: 'Compare latest', exact: true })
      .click();
    await expect(
      candidateEditor.locator('.cm-mergeViewEditor').last(),
    ).toContainText('# Accepted candidate edit');
    await page.screenshot({
      path: `${root}/yaml-candidate-conflict.png`,
      fullPage: true,
    });
    await candidateEditor
      .getByRole('button', { name: 'Reload latest files', exact: true })
      .click();
    await page
      .getByRole('dialog', { name: 'Reload latest files?', exact: true })
      .getByRole('button', { name: 'Reload latest files', exact: true })
      .click();
    await candidateEditor
      .getByRole('tab', { name: 'recyclarr.yml', exact: true })
      .click();
    await expect(candidateEditor.locator('.cm-content')).toContainText(
      '# Accepted candidate edit',
    );
    await candidateEditor
      .getByRole('button', { name: 'Close YAML editor', exact: true })
      .click();
  }
  await preflight('blocked');
  const blocked = await api('/admin/recyclarr/configuration');
  expect(blocked.candidate.valid).toBe(false);
  expect(blocked.files).toEqual(customFiles);
  expect(blocked.image).toBe(customized.image);
  const replacedCandidate = await request(
    '/admin/recyclarr/configuration/candidate',
    'POST',
    {
      revision: customized.revision,
      candidate_revision: edited.candidate.revision,
      files: customFiles,
    },
  );
  expect(replacedCandidate.status()).toBe(409);
  const corrected = {
    ...customFiles,
    'includes/retained.yml':
      '# Reviewed candidate change\ndelete_old_custom_formats: false\n',
  };
  await api('/admin/recyclarr/configuration/candidate', 'POST', {
    revision: customized.revision,
    candidate_revision: blocked.candidate.revision,
    files: corrected,
  });
  const accepted = await preflight();
  await api(`/admin/service-updates/${accepted.id}/activate`, 'POST', {});
  await waitUpdate(accepted.id, 'committed');
  const current = await api('/admin/recyclarr/configuration');
  expect(current.mode).toBe('customized');
  expect(current.files).toEqual(corrected);
  expect(current.defaults.image).toBe(current.image);
  for (const kind of ['radarr', 'sonarr'])
    expect(await snapshot(kind)).toEqual(before[kind]);
  evidence.configuration_upgrades = {
    default_revision: upgraded.revision,
    customized_revision: current.revision,
    candidate_files: Object.keys(corrected),
  };
  record(
    'Candidate qualification checks settings.yml, blocks invalid candidates without changing active files or Arr, and activates only the reviewed custom file set',
  );
  record(
    'Candidate saves need no live managers; competing edits and replaced candidate operations reject stale revisions without overwriting the accepted draft',
  );
  await api('/admin/recyclarr/configuration/defaults', 'POST', {
    revision: current.revision,
  });
}
