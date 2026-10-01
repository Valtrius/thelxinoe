import { cleanupResources } from './ci-resources.mjs';

const directory = process.env.THELXINOE_CI_RESOURCE_DIRECTORY;
const owner = process.env.THELXINOE_CI_RUN_ID;
if (!directory || !owner)
  throw Error('CI resource directory and run ID are required');
const errors = cleanupResources(directory, owner, { removeImages: true });
if (errors.length) {
  console.error(errors.join('\n'));
  process.exitCode = 1;
}
