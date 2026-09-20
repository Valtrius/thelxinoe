// Copies only source/build inputs into a private context. Production version is unchanged.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, copyFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
const root = '.local/release-fixture';
const files = execFileSync(
  'git',
  ['ls-files', '--cached', '--others', '--exclude-standard'],
  { encoding: 'utf8' },
)
  .trim()
  .split(/\r?\n/);
for (const file of new Set(files)) {
  if (
    !/^(apps\/|crates\/|frontend\/|scripts\/|releases\/|Cargo\.|Dockerfile$|compose\.|package|\.dockerignore$)/.test(
      file,
    )
  )
    continue;
  const dest = join(root, file);
  mkdirSync(dirname(dest), { recursive: true });
  copyFileSync(file, dest);
}
function edit(path, change) {
  const file = join(root, path);
  writeFileSync(file, change(readFileSync(file, 'utf8')));
}
edit('Cargo.toml', (s) => s.replace('version = "0.1.0"', 'version = "0.2.0"'));
edit('Cargo.lock', (s) =>
  s.replace(/(name = "thelxinoe-[^"]+"\r?\nversion = ")0\.1\.0/g, '$10.2.0'),
);
for (const file of [
  'package.json',
  'frontend/package.json',
  'apps/desktop/tauri.conf.json',
])
  edit(file, (s) => {
    const v = JSON.parse(s);
    v.version = '0.2.0';
    return JSON.stringify(v, null, 2) + '\n';
  });
for (const file of ['compose.yaml', 'compose.test.yaml'])
  edit(file, (s) =>
    s.replace(/thelxinoe-(server|controller):0\.1\.0/g, 'thelxinoe-$1:0.2.0'),
  );
edit('package-lock.json', (s) => {
  const v = JSON.parse(s);
  v.version = '0.2.0';
  v.packages[''].version = '0.2.0';
  v.packages.frontend.version = '0.2.0';
  return JSON.stringify(v, null, 2) + '\n';
});
const schema = Number(
  readFileSync('crates/database/src/lib.rs', 'utf8').match(
    /SCHEMA_VERSION: u32 = (\d+)/,
  )[1],
);
const nextSchema = schema + 1;
const migration = (n) => String(n).padStart(3, '0');
edit('crates/database/src/lib.rs', (s) =>
  s
    .replace(
      `SCHEMA_VERSION: u32 = ${schema}`,
      `SCHEMA_VERSION: u32 = ${nextSchema}`,
    )
    .replace(
      `include_str!("../migrations/${migration(schema)}.sql"),`,
      `include_str!("../migrations/${migration(schema)}.sql"),\n            include_str!("../migrations/${migration(nextSchema)}.sql"),`,
    ),
);
writeFileSync(
  join(root, `crates/database/migrations/${migration(nextSchema)}.sql`),
  'CREATE TABLE release_forward_only_fixture(id INTEGER PRIMARY KEY, value TEXT NOT NULL);\n',
);
edit('apps/server/src/validation.rs', (s) =>
  s.replace(
    '    let report =',
    '    if state.config.state.join("hold-live-validation").exists() { std::fs::write(state.config.state.join("held-validation-entered"), b"migrated")?; tokio::time::sleep(std::time::Duration::from_secs(60)).await; }\n    anyhow::ensure!(!state.config.state.join("fail-live-validation").exists(), "Deliberate failure after a forward-only test migration");\n    let report =',
  ),
);
console.log(
  `Prepared private 0.2.0 release with schema ${nextSchema} and a controlled migration failure fixture`,
);
