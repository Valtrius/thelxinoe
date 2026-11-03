import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { writeFileSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';

// Listed in the order the controller hashes them into a resource revision.
const repositories = [
  {
    path: 'TRaSH-Guides/Guides',
    commit: 'd0f1a6cb29bee8b53bd1d7f8c73333649972f21c',
  },
  {
    path: 'recyclarr/config-templates',
    commit: '89eebadd429952069182a22930af4d768cacda36',
  },
];

export const guideRevision = createHash('sha256')
  .update(repositories.map(({ commit }) => commit).join(''))
  .digest('hex');

const git = (...args) =>
  execFileSync('git', args, {
    timeout: 120000,
    killSignal: 'SIGKILL',
    windowsHide: true,
    stdio: 'pipe',
  });

function mirror(directory, { path, commit }) {
  for (let attempt = 1; ; attempt++) {
    try {
      rmSync(directory, { recursive: true, force: true });
      git('init', '-q', '--bare', '-b', 'master', directory);
      git(
        '-C',
        directory,
        'fetch',
        '-q',
        '--depth',
        '1',
        `https://github.com/${path}.git`,
        commit,
      );
      git('-C', directory, 'update-ref', 'refs/heads/master', commit);
      return;
    } catch (error) {
      if (attempt === 4) throw error;
      console.log(`Retrying the ${path} mirror: ${error.message}`);
    }
  }
}

/**
 * Fill a directory for THELXINOE_RECYCLARR_GUIDE_MIRROR, so a Recyclarr container
 * fetches the pinned commits instead of GitHub's current ones. The repositories
 * have another owner inside the container, which `safe.directory` allows.
 */
export function mirrorGuides(directory) {
  const root = resolve(directory);
  for (const repository of repositories)
    mirror(join(root, `${repository.path}.git`), repository);
  writeFileSync(
    join(root, 'gitconfig'),
    '[url "file:///guide-mirror/"]\n\tinsteadOf = https://github.com/\n[safe]\n\tdirectory = *\n',
  );
  return root;
}
