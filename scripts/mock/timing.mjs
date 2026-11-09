// How long server actions take by default, so their pending states show.
// Tests start without it and set a delay on the services that need one.
export const serverActionDelay = Number(
  process.env.THELXINOE_MOCK_ACTION_DELAY ?? 3000,
);
if (![0, 500, 1500, 3000, 5000].includes(serverActionDelay))
  throw Error('Invalid THELXINOE_MOCK_ACTION_DELAY');
/** How long each transfer step takes. */
export const transferStepDelay = 3000;

export async function waitForServerAction(s, { path, method, body }) {
  const write = !['GET', 'HEAD'].includes(method);
  const action =
    (write &&
      (path.startsWith('/admin/') ||
        path.startsWith('/seerr/requests') ||
        /^\/online\/[^/]+\/.*(?:download|refresh|sync)/.test(path) ||
        path === '/playback' ||
        /^\/playback\/[^/]+\/seek$/.test(path))) ||
    (method === 'GET' &&
      (/^\/admin\/transfers\/[^/]+\/recovery$/.test(path) ||
        /^\/catalog\/[^/]+\/playback$/.test(path)));
  if (!action) return;

  const id = path.split('/')[3];
  const link = s.connections.find(
    (item) => item.id === id || item.source_id === body.source_id,
  );
  const kind =
    body.kind ??
    s.services.find((item) => item.id === id)?.kind ??
    s.stack.find((item) => item.id === id)?.kind ??
    s.transfers?.[id]?.kind ??
    link?.source_kind;
  const delay = s.scenarios?.services[kind]?.delay ?? serverActionDelay;
  if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
}
