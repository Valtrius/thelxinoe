// Real-container qualification of Connected → Owned ownership transfers.
//
// Every scenario starts an externally managed service from the curated image, connects
// it, takes ownership through the public API and checks the original, the copy and the
// Owned replacement. The result is the transfer qualification artifact.
import { expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { docker, fixture } from './service-access-fixture.mjs';
import { imageManifest } from './ci-images.mjs';

const output = 'test-results/transfers';
mkdirSync(output, { recursive: true });
const option = (name) =>
  process.argv
    .find((value) => value.startsWith(`--${name}=`))
    ?.slice(name.length + 3)
    .split(',');
const kinds = option('kinds') ?? [
  'radarr',
  'sonarr',
  'lidarr',
  'prowlarr',
  'bazarr',
  'nzbget',
  'seerr',
];
const storages = option('storage') ?? ['bind', 'volume'];
const scenarios = option('scenarios') ?? ['matrix', 'interruption', 'recovery'];
const result = {
  passed: false,
  started_at: new Date().toISOString(),
  images: imageManifest.services,
  matrix: [],
  scenarios: [],
};
const save = () =>
  writeFileSync(`${output}/result.json`, JSON.stringify(result, null, 2));
const internalPort = {
  radarr: 7878,
  sonarr: 8989,
  lidarr: 8686,
  prowlarr: 9696,
  bazarr: 6767,
  nzbget: 6789,
};
const apiVersion = { radarr: 3, sonarr: 3, lidarr: 1, prowlarr: 1 };
const inspect = (container) => JSON.parse(docker('inspect', container))[0];

// Each kind can be Owned once per server, so every pass gets a fresh deployment.
let f;
async function controllerLogs(name) {
  try {
    writeFileSync(
      `${output}/${name}-services.log`,
      f.compose('logs', '--no-color', '--tail', '400', 'server', 'controller'),
    );
  } catch (error) {
    console.error('Could not capture transfer diagnostics:', error.message);
  }
}
async function stage(kind) {
  return (await f.api('/admin/services')).items.find(
    (item) => item.kind === kind,
  );
}
const retry = async (label, run, attempts = 4) => {
  for (let attempt = 1; ; attempt++) {
    try {
      return await run();
    } catch (error) {
      // Library records need the services' public metadata APIs.
      if (attempt >= attempts)
        throw Error(`${label}: ${error.message}`, { cause: error });
      await new Promise((resolve) => setTimeout(resolve, 5000 * attempt));
    }
  }
};
const pick = (rows, fields) =>
  rows.map((row) => Object.fromEntries(fields.map((key) => [key, row[key]])));
/** Write a media file through the original, which already mounts the media root. */
function mediaFile(source, path) {
  docker(
    'exec',
    '-u',
    '0',
    source.container,
    'sh',
    '-c',
    // Only the new title folder; other media keeps the original's ownership.
    'mkdir -p "$(dirname "$1")" && head -c 2000000 /dev/urandom > "$1" && chown -R 10001:10001 "$(dirname "$1")"',
    'sh',
    path,
  );
}
/**
 * Populate the originals while they are Connected and externally managed. The records
 * span library items with physical files, profiles, custom formats and cross-service
 * client/application records whose identities must survive in each replacement.
 */
async function populate(sources) {
  const records = {};
  for (const kind of ['radarr', 'sonarr', 'lidarr', 'prowlarr']) {
    const source = sources[kind];
    if (!source) continue;
    const tag = await source.direct('tag', 'POST', {
      label: `transfer-${kind}`,
    });
    records[kind] = { tag: [{ id: tag.id, label: tag.label }] };
  }
  const nzbget = sources.nzbget;
  const client = (category) => ({
    name: 'Fixture NZBGet',
    implementation: 'Nzbget',
    configContract: 'NzbgetSettings',
    protocol: 'usenet',
    enable: true,
    priority: 1,
    removeCompletedDownloads: true,
    removeFailedDownloads: true,
    fields: [
      { name: 'host', value: nzbget.name },
      { name: 'port', value: 6789 },
      { name: 'username', value: 'fixture' },
      { name: 'password', value: nzbget.key },
      // Arr validates the category against NZBGet's own list.
      { name: category, value: 'existing-1' },
    ],
    tags: [],
  });
  if (sources.radarr) {
    const radarr = sources.radarr;
    const root = await radarr.direct('rootfolder', 'POST', {
      path: '/media/movies',
    });
    const format = await radarr.direct('customformat', 'POST', {
      name: 'Transfer format',
      includeCustomFormatWhenRenaming: false,
      specifications: [
        {
          name: 'Title',
          implementation: 'ReleaseTitleSpecification',
          negate: false,
          required: false,
          fields: [{ name: 'value', value: 'transfer' }],
        },
      ],
    });
    // The file exists first, so Radarr's own refresh after adding imports it once.
    mediaFile(
      radarr,
      '/media/movies/The Matrix (1999)/The.Matrix.1999.1080p.WEB-DL.mkv',
    );
    const movie = await retry('Radarr movie metadata', () =>
      radarr.direct('movie', 'POST', {
        title: 'The Matrix',
        titleSlug: 'the-matrix-603',
        tmdbId: 603,
        year: 1999,
        images: [],
        path: '/media/movies/The Matrix (1999)',
        minimumAvailability: 'released',
        qualityProfileId: 1,
        monitored: true,
        tags: [records.radarr.tag[0].id],
        addOptions: { searchForMovie: false },
      }),
    );
    const files = `moviefile?movieId=${movie.id}`;
    await expect
      .poll(async () => (await radarr.direct(files)).length, {
        timeout: 90000,
      })
      .toBeGreaterThan(0);
    Object.assign(records.radarr, {
      rootfolder: pick([root], ['id', 'path']),
      customformat: pick([format], ['id', 'name']),
      movie: pick([movie], ['id', 'tmdbId', 'monitored']),
      [files]: pick(await radarr.direct(files), [
        'id',
        'movieId',
        'relativePath',
      ]),
    });
    if (nzbget)
      records.radarr.downloadclient = pick(
        [
          await radarr.direct(
            'downloadclient',
            'POST',
            client('movieCategory'),
          ),
        ],
        ['id', 'name'],
      );
  }
  if (sources.sonarr) {
    const sonarr = sources.sonarr;
    const root = await sonarr.direct('rootfolder', 'POST', {
      path: '/media/tv',
    });
    const series = await retry('Sonarr series metadata', () =>
      sonarr.direct('series', 'POST', {
        title: 'Fixture Series',
        titleSlug: 'fixture-series',
        tvdbId: 121361,
        year: 2011,
        images: [],
        seasons: [],
        path: '/media/tv/Fixture Series',
        seriesType: 'standard',
        qualityProfileId: 1,
        monitored: true,
        seasonFolder: true,
        addOptions: { searchForMissingEpisodes: false },
      }),
    );
    Object.assign(records.sonarr, {
      rootfolder: pick([root], ['id', 'path']),
      series: pick([series], ['id', 'tvdbId', 'monitored']),
    });
    if (nzbget)
      records.sonarr.downloadclient = pick(
        [await sonarr.direct('downloadclient', 'POST', client('tvCategory'))],
        ['id', 'name'],
      );
  }
  if (sources.seerr) {
    // The initialized administrator and a local member must keep their identities.
    const member = await sources.seerr.direct('user', 'POST', {
      email: 'member@example.invalid',
      username: 'Fixture member',
      // Without a password Seerr would email one.
      password: 'fixture member passphrase',
      permissions: 32,
    });
    records.seerr = {
      // Seerr's create response omits the email it stored.
      'user?take=50': [
        { id: 1, email: 'fixture-admin@example.invalid' },
        { id: member.id, email: 'member@example.invalid' },
      ],
    };
  }
  if (sources.prowlarr && sources.radarr) {
    const prowlarr = sources.prowlarr;
    const application = await prowlarr.direct('applications', 'POST', {
      name: 'Fixture Radarr',
      implementation: 'Radarr',
      configContract: 'RadarrSettings',
      syncLevel: 'disabled',
      fields: [
        { name: 'prowlarrUrl', value: `http://${prowlarr.name}:9696` },
        { name: 'baseUrl', value: `http://${sources.radarr.name}:7878` },
        { name: 'apiKey', value: sources.radarr.key },
        { name: 'syncCategories', value: [2000] },
      ],
      tags: [],
    });
    records.prowlarr.applications = pick([application], ['id', 'name']);
  }
  return records;
}
/** Read the replacement's own API from inside its network namespace. */
function replacementApi(kind, container, key, path) {
  // Seerr has no URL Base and its image ships wget rather than curl.
  if (kind === 'seerr')
    return JSON.parse(
      docker(
        'exec',
        container,
        'wget',
        '-qO-',
        '--header',
        `X-Api-Key: ${key}`,
        `http://localhost:5055/api/v1/${path}`,
      ),
    );
  const version = apiVersion[kind];
  return JSON.parse(
    docker(
      'exec',
      container,
      'curl',
      '-fsS',
      '-H',
      `X-Api-Key: ${key}`,
      `http://localhost:${internalPort[kind]}/services/${kind}/api/v${version}/${path}`,
    ),
  );
}
async function matrix(source, records, storage) {
  const kind = source.kind;
  const name = `${kind}-${storage}`;
  const started = Date.now();
  const entry = { kind, storage, image: imageManifest.services[kind] };
  result.matrix.push(entry);
  try {
    expect((await stage(kind)).stage).toBe('connected');
    // Folders the original created as another identity may be unusable by the
    // deployment's. The review names them before anything stops; Thelxinoe never
    // re-owns media, so the administrator adjusts permissions and reviews again.
    const first = await f.transfer(source.id, { approve: false });
    const permissions = first.review.review.blockers.filter((blocker) =>
      /isn't (writable|readable|reachable) by/.test(blocker),
    );
    if (permissions.length) {
      expect(inspect(source.container).State.Running).toBe(true);
      entry.permission_blockers = permissions;
      docker(
        'exec',
        '-u',
        '0',
        source.container,
        'chmod',
        '-R',
        'a+rwX',
        '/media',
      );
    }
    const { view, review } = await f.transfer(source.id);
    const { plan } = review.review;
    entry.review = {
      source: {
        image: plan.source.image,
        version: plan.source.version,
        identity: plan.source.identity,
        appdata: plan.source.appdata,
      },
      target: {
        image: plan.target.image,
        version: plan.target.version,
        identity: plan.target.identity,
        mounts: plan.target.mounts,
        supplementary: plan.target.supplementary,
      },
      decision: plan.target.decision,
      changes: plan.changes,
      settings: plan.settings,
    };
    entry.state = view.state;
    entry.checks = view.checks;
    entry.controller = view.controller;
    if (view.state !== 'complete')
      throw Error(`${name} ended ${view.state}: ${view.error ?? 'no error'}`);
    for (const check of view.checks.filter((check) => check.required))
      expect(check.status, `${name} ${check.id}`).toBe('passed');
    // The original stays stopped with its restart policy disabled.
    const original = inspect(source.container);
    expect(original.State.Running).toBe(false);
    expect(original.HostConfig.RestartPolicy.Name).toBe('no');
    // Settings change only in the copy; the original keeps its own.
    if (apiVersion[kind])
      expect(f.readAppdata(source.appdata, 'config.xml')).toContain(
        '<UrlBase></UrlBase>',
      );
    const owned = await stage(kind);
    expect(owned).toMatchObject({
      stage: 'owned',
      hold: null,
      ownership: { origin: 'transferred' },
    });
    expect(owned.connection.operational).toBe(true);
    const replacement = (await f.stack()).items.find(
      (item) => item.kind === kind,
    );
    expect(replacement?.running).toBe(true);
    expect(replacement.container_id).not.toBe(source.container);
    // Library, profile and client identities are the same records in the replacement.
    for (const [path, rows] of Object.entries(records ?? {})) {
      const response = replacementApi(
        kind,
        replacement.container_id,
        source.key,
        path,
      );
      // Seerr pages its lists under results.
      const observed = [response?.results ?? response].flat();
      for (const row of rows)
        expect(observed, `${name} ${path}`).toContainEqual(
          expect.objectContaining(row),
        );
    }
    entry.preserved = records ?? {};
    // In a partially transferred group, links join only operational Owned services.
    const stages = (await f.api('/admin/services')).items;
    const operational = new Set(
      stages
        .filter((item) => item.connection?.operational)
        .map((item) => item.connection.id),
    );
    const links = (await f.api('/admin/service-connections')).items;
    for (const link of links.filter((link) => link.enabled))
      expect(
        operational.has(link.source_id) && operational.has(link.target_id),
        `${name} link ${link.source_id} → ${link.target_id}`,
      ).toBe(true);
    entry.links = links.map((link) => ({
      source: link.source_id,
      target: link.target_id,
      enabled: link.enabled,
      state: link.state,
    }));
    // Removing the retained original deletes its container, never its appdata.
    const before = f.appdataDigest(source.appdata);
    await f.api(`/admin/transfers/${view.id}/original/remove`, 'POST');
    expect(() => inspect(source.container)).toThrow();
    expect(f.appdataDigest(source.appdata)).toBe(before);
    entry.passed = true;
  } catch (error) {
    entry.passed = false;
    entry.error = error.message;
    await controllerLogs(name);
    throw error;
  } finally {
    entry.duration_ms = Date.now() - started;
    save();
  }
}
async function scenario(name, run) {
  const started = Date.now();
  try {
    await run();
    result.scenarios.push({
      name,
      passed: true,
      duration_ms: Date.now() - started,
    });
    console.log(`PASS ${name}`);
  } catch (error) {
    result.scenarios.push({ name, passed: false, error: error.message });
    await controllerLogs(name.replace(/\W+/g, '-'));
    throw error;
  } finally {
    save();
  }
}

async function pass(run) {
  f = await fixture({ scheme: 'http' });
  try {
    await run();
  } finally {
    await f.close();
    f = undefined;
  }
}

try {
  if (scenarios.includes('matrix'))
    for (const storage of storages)
      await pass(async () => {
        // Every original is connected and cross-linked before any transfer starts.
        const sources = {};
        for (const kind of kinds)
          sources[kind] = await f.attach(
            kind,
            '',
            imageManifest.services[kind],
            {
              volume: storage === 'volume',
              // Both supported source identities differ from the deployment's.
              identity: storage === 'volume' ? '1001:1001' : '1000:1000',
            },
          );
        const records = await populate(sources);
        let disconnected;
        for (const kind of kinds) {
          await matrix(sources[kind], records[kind], storage);
          console.log(`PASS transfer ${kind} (${storage})`);
          // A manual disconnect between Owned services survives later takeovers.
          if (kind === 'bazarr' && sources.radarr) {
            disconnected = {
              source_id: sources.bazarr.id,
              target_id: sources.radarr.id,
            };
            await f.api('/admin/service-connections', 'POST', {
              ...disconnected,
              action: 'disconnect',
            });
          }
        }
        if (disconnected) {
          const link = (await f.api('/admin/service-connections')).items.find(
            (item) =>
              item.source_id === disconnected.source_id &&
              item.target_id === disconnected.target_id,
          );
          expect(link?.enabled).toBe(false);
          result.scenarios.push({
            name: `a manual disconnect survives later takeovers (${storage})`,
            passed: true,
          });
        }
      });
  await pass(async () => {
    const stepIndex = (step) =>
      [
        'stop',
        'copy',
        'validate',
        'activate',
        'commit',
        'setup',
        'checks',
        'complete',
      ].indexOf(step);
    /** Approve a transfer and return once it reaches the given step or ends. */
    async function approveUntil(source, step) {
      const { view } = await f.transfer(source.id, {
        until: ['queued', 'running', 'committed', 'complete', 'failed'],
      });
      return f.waitTransfer(
        view.id,
        ['complete', 'failed', 'suspended', 'returned', 'cancelled'],
        undefined,
        (current) => stepIndex(current.step) >= stepIndex(step),
      );
    }
    if (scenarios.includes('interruption'))
      for (const [kind, step, component] of [
        ['sonarr', 'copy', 'controller'],
        ['lidarr', 'validate', 'controller'],
        ['bazarr', 'activate', 'server'],
        ['nzbget', 'setup', 'server'],
      ])
        await scenario(
          `a ${component} restart during ${step} resumes the ${kind} transfer from its journal`,
          async () => {
            const source = await f.attach(
              kind,
              '',
              imageManifest.services[kind],
            );
            const reached = await approveUntil(source, step);
            f.compose('restart', component);
            const finished = await f.waitTransfer(reached.id);
            expect(finished.state, finished.error ?? '').toBe('complete');
            // Exactly one replacement exists for the operation.
            expect(
              docker(
                'ps',
                '-aq',
                '--filter',
                `label=app.thelxinoe.managed-id=${reached.id}`,
              )
                .split(/\s+/)
                .filter(Boolean),
            ).toHaveLength(1);
            expect(inspect(source.container).State.Running).toBe(false);
          },
        );
    if (scenarios.includes('recovery')) {
      await scenario('a stale review never stops the original', async () => {
        const source = await f.attach(
          'prowlarr',
          '',
          imageManifest.services.prowlarr,
        );
        const { review } = await f.transfer(source.id, { approve: false });
        await f.api(`/admin/services/${source.id}/connection`, 'PUT', {
          port: internalPort.prowlarr,
          url_base: '',
          secret: source.key,
        });
        const response = await f.context.request.post(
          `${f.base}/api/v1/admin/transfers/${review.id}/approve`,
          {
            headers: { 'X-Thelxinoe-Client': '1' },
            data: {
              review_revision: review.review_revision,
              confirm: true,
              declarations: {
                cron: true,
                systemd: true,
                ci_cd: true,
                configuration_management: true,
                manual_compose: true,
              },
            },
          },
        );
        expect(response.status()).toBe(409);
        expect(inspect(source.container).State.Running).toBe(true);
        expect((await stage('prowlarr')).stage).toBe('connected');
        await f.api(`/admin/services/${source.id}/forget`, 'POST');
        docker('rm', '-f', source.container);
      });
      await scenario(
        'a failure before activation restores the original automatically',
        async () => {
          const source = await f.attach(
            'prowlarr',
            '',
            imageManifest.services.prowlarr,
          );
          const policy = inspect(source.container).HostConfig.RestartPolicy;
          const reached = await approveUntil(source, 'validate');
          // Lose the disposable candidate while it is being validated.
          await expect
            .poll(
              () =>
                docker(
                  'ps',
                  '-q',
                  '--filter',
                  `label=app.thelxinoe.transfer=${reached.id}`,
                  '--filter',
                  'name=-candidate',
                ),
              { timeout: 120000, intervals: [250] },
            )
            .not.toBe('');
          docker(
            'kill',
            docker(
              'ps',
              '-q',
              '--filter',
              `label=app.thelxinoe.transfer=${reached.id}`,
              '--filter',
              'name=-candidate',
            ),
          );
          const finished = await f.waitTransfer(reached.id);
          expect(finished.controller?.boundary_at ?? null).toBeNull();
          await expect
            .poll(() => inspect(source.container).State.Running, {
              timeout: 90000,
            })
            .toBe(true);
          expect(inspect(source.container).HostConfig.RestartPolicy).toEqual(
            policy,
          );
          expect((await stage('prowlarr')).stage).toBe('connected');
          result.recovery = { pre_activation: finished };
          await f.api(`/admin/services/${source.id}/forget`, 'POST');
          docker('rm', '-f', source.container);
        },
      );
      await scenario(
        'a failure after activation stays held until an approved return to the original',
        async () => {
          const source = await f.attach(
            'radarr',
            '',
            imageManifest.services.radarr,
          );
          const reached = await approveUntil(source, 'activate');
          // The replacement disappears after the production boundary.
          await expect
            .poll(
              async () =>
                (await f.api(`/admin/transfers/${reached.id}`)).controller
                  ?.boundary_at ?? null,
              { timeout: 300000, intervals: [250] },
            )
            .not.toBeNull();
          const replacement = () =>
            docker(
              'ps',
              '-aq',
              '--filter',
              `label=app.thelxinoe.managed-id=${reached.id}`,
            );
          await expect
            .poll(replacement, { timeout: 120000, intervals: [250] })
            .not.toBe('');
          docker('kill', replacement());
          const failed = await f.waitTransfer(reached.id);
          expect(failed.state).toBe('failed');
          expect(failed.controller.boundary_at).not.toBeNull();
          // After the boundary nothing is operational and the original stays stopped.
          // A committed replacement is Owned under hold; before commit it stays Connected.
          const held = await stage('radarr');
          expect(held.connection?.operational ?? false).toBe(false);
          expect(held.operation).toMatchObject({
            type: 'transfer',
            state: 'failed',
          });
          if (failed.controller.committed)
            expect(held).toMatchObject({ stage: 'owned', hold: 'transfer' });
          else expect(held.stage).toBe('connected');
          expect(inspect(source.container).State.Running).toBe(false);
          const plan = await f.api(`/admin/transfers/${reached.id}/recovery`);
          expect(plan.cannot_undo.length).toBeGreaterThan(0);
          const back = plan.options.find(
            (option) => option.action === 'return_original',
          );
          expect(back.available).toBe(true);
          // Without explicit approval of its consequences nothing changes.
          await expect(
            f.api(`/admin/transfers/${reached.id}/recovery`, 'POST', {
              action: 'return_original',
              revision: plan.revision,
              approval: {},
            }),
          ).rejects.toThrow(/HTTP 4\d\d/);
          await f.api(`/admin/transfers/${reached.id}/recovery`, 'POST', {
            action: 'return_original',
            revision: plan.revision,
            approval: Object.fromEntries(
              back.requires.map((requirement) => [requirement, true]),
            ),
          });
          const returned = await f.waitTransfer(reached.id, [
            'returned',
            'failed',
            'suspended',
          ]);
          expect(returned.state, returned.error ?? '').toBe('returned');
          expect(inspect(source.container).State.Running).toBe(true);
          expect((await stage('radarr')).stage).toBe('connected');
          // The returned connection is the original's again, storage mounts included,
          // once the restarted original finishes starting.
          await expect
            .poll(
              async () =>
                (
                  await f
                    .api(`/admin/services/${source.id}/test`, 'POST')
                    .catch((error) => ({ healthy: error.message }))
                ).healthy,
              { timeout: 180000 },
            )
            .toBe(true);
          result.recovery = {
            ...result.recovery,
            post_activation: { plan, outcome: returned.state },
          };
        },
      );
    }
  });
  result.passed = true;
} finally {
  result.finished_at = new Date().toISOString();
  save();
  writeFileSync(
    '.local/transfer-qualification.json',
    JSON.stringify(result, null, 2),
  );
}
