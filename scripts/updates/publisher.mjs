import { createServer } from 'node:https';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { sign } from 'node:crypto';
import { compile } from '@tailwindcss/node';
import { repository } from './build.mjs';

const lab = JSON.parse(readFileSync(process.argv[2], 'utf8'));
let mode = 'base';
const modes = [
  'base',
  'candidate',
  'unavailable',
  'tampered',
  'expired',
  'mismatch',
  'corrupt',
];
const page = `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Thelxinoe update lab</title><style>STYLES</style><body class="bg-slate-950 text-slate-100 p-6"><main class="max-w-3xl mx-auto grid gap-6"><h1 class="text-3xl font-semibold">Thelxinoe update lab</h1><p>Base ${lab.base} → candidate ${lab.next}. Each choice changes the signed feed served to this lab.</p><a class="text-sky-300 underline" href="${lab.baseUrl}" target="_blank" rel="noreferrer">Open the test server</a><p>Sign in with <strong>admin</strong> and the test password printed by the lab command.</p><form class="grid gap-3"><label for="mode">Publisher response</label><select id="mode" class="bg-slate-800 border border-slate-600 p-3">${modes.map((v) => `<option>${v}</option>`).join('')}</select><button class="bg-sky-300 text-slate-950 px-4 py-3 font-semibold" type="submit">Apply publisher response</button></form><p role="status" id="status"></p><p>Use Check server release or Check desktop release after changing the publisher. Base withdraws the candidate. Unavailable returns HTTP 503; tampered changes the signed envelope; expired signs an expired envelope; mismatch changes installer metadata; corrupt changes installer bytes.</p><p>Preparation briefly stops the server. Installation replaces the server and controller. Keep the web tab open to observe reconnection and recovery.</p><p>Run the printed reset command to start again from the base version. All installations and keys belong to this lab.</p></main><script>const status=document.querySelector('#status');fetch('/status').then(r=>r.json()).then(v=>{document.querySelector('#mode').value=v.mode;status.textContent='Serving '+v.mode;});document.querySelector('form').onsubmit=async e=>{e.preventDefault();const r=await fetch('/control',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({mode:document.querySelector('#mode').value})});status.textContent=r.ok?'Serving '+document.querySelector('#mode').value:'Publisher change failed';};</script></body></html>`;
const compiler = await compile('@import "tailwindcss";', {
  base: repository,
  onDependency: () => {},
});
const html = page.replace(
  'STYLES',
  compiler.build(
    [...page.matchAll(/class="([^"]*)"/g)].flatMap((m) => m[1].split(' ')),
  ),
);
const server = createServer(
  {
    key: readFileSync(join(lab.root, 'tls.key')),
    cert: readFileSync(join(lab.root, 'tls.crt')),
  },
  async (request, response) => {
    response.setHeader('Cache-Control', 'no-store');
    const url = new URL(request.url, lab.publisher);
    const local = ['127.0.0.1', '::1', '::ffff:127.0.0.1'].includes(
      request.socket.remoteAddress,
    );
    if (url.pathname === '/control' && request.method === 'POST') {
      if (
        !local ||
        (request.headers.origin && request.headers.origin !== lab.publisher)
      ) {
        response.writeHead(403).end();
        return;
      }
      let data = '';
      for await (const chunk of request) {
        data += chunk;
        if (data.length > 1024) {
          response.writeHead(413).end();
          return;
        }
      }
      try {
        const value = JSON.parse(data);
        if (!modes.includes(value.mode)) throw Error('Invalid mode');
        mode = value.mode;
        response
          .writeHead(200, { 'Content-Type': 'application/json' })
          .end(JSON.stringify({ mode }));
      } catch {
        response.writeHead(400).end();
      }
      return;
    }
    if (request.method !== 'GET') {
      response.writeHead(405).end();
      return;
    }
    if (url.pathname === '/') {
      response
        .writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' })
        .end(html);
      return;
    }
    if (url.pathname === '/status') {
      response
        .writeHead(200, { 'Content-Type': 'application/json' })
        .end(JSON.stringify({ mode, id: lab.id, pid: process.pid }));
      return;
    }
    // Exercise HTTPS redirection in both native and server discovery.
    if (url.pathname === '/latest.json') {
      response.writeHead(302, { Location: '/channel/latest.json' }).end();
      return;
    }
    try {
      let bytes;
      if (url.pathname === '/channel/latest.json') {
        if (mode === 'unavailable') {
          response.writeHead(503).end();
          return;
        }
        const release = mode === 'base' ? lab.base : lab.next;
        bytes = readFileSync(join(lab.root, 'channel', release, 'latest.json'));
        if (mode === 'tampered') {
          const value = JSON.parse(bytes);
          const payload = JSON.parse(Buffer.from(value.payload, 'base64'));
          payload.notes += ' modified';
          value.payload = Buffer.from(JSON.stringify(payload)).toString(
            'base64',
          );
          bytes = Buffer.from(JSON.stringify(value));
        }
        if (mode === 'expired') {
          const value = JSON.parse(bytes);
          const payload = JSON.parse(Buffer.from(value.payload, 'base64'));
          payload.published_at = Math.floor(Date.now() / 1000) - 120;
          payload.expires_at = payload.published_at + 60;
          const raw = Buffer.from(JSON.stringify(payload));
          bytes = Buffer.from(
            JSON.stringify({
              payload: raw.toString('base64'),
              signature: sign(
                null,
                Buffer.concat([
                  Buffer.from('Thelxinoe release manifest v1\0'),
                  raw,
                ]),
                readFileSync(join(lab.root, 'signing.pem')),
              ).toString('base64'),
            }),
          );
        }
      } else {
        const match = url.pathname.match(
          /^\/releases\/(\d+\.\d+\.\d+)\/(windows-x64\.json|setup\.exe(?:\.sig)?)$/,
        );
        if (!match || ![lab.base, lab.next].includes(match[1])) {
          response.writeHead(404).end();
          return;
        }
        bytes = readFileSync(join(lab.root, 'channel', match[1], match[2]));
        if (mode === 'mismatch' && match[2] === 'windows-x64.json') {
          const value = JSON.parse(bytes);
          value.platforms['windows-x86_64'].signature =
            'mismatched-lab-signature';
          bytes = Buffer.from(JSON.stringify(value));
        }
        if (mode === 'corrupt' && match[2] === 'setup.exe') {
          bytes = Buffer.from(bytes);
          bytes[bytes.length - 1] ^= 1;
        }
      }
      response
        .writeHead(200, {
          'Content-Type': url.pathname.endsWith('.json')
            ? 'application/json'
            : 'application/octet-stream',
          'Content-Length': bytes.length,
        })
        .end(bytes);
    } catch {
      response.writeHead(404).end();
    }
  },
);
server.listen(lab.publisherPort, '0.0.0.0', () =>
  writeFileSync(join(lab.root, 'publisher-ready'), String(process.pid)),
);
