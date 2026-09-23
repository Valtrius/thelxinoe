import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { compile } from 'tailwindcss';

const require = createRequire(import.meta.url);

// Cache only the public connection page. Its Tailwind output is embedded so it
// remains styled offline without retaining application code or hashed assets.
/** @returns {import('vite').Plugin} */
export function offlinePage() {
  return {
    name: 'thelxinoe-offline-page',
    transformIndexHtml: {
      order: 'pre',
      async handler(html, context) {
        if (context.path !== '/offline.html') return html;
        const compiler = await compile(
          "@import 'tailwindcss/index.css' source(none);",
          {
            base: dirname(context.filename),
            async loadStylesheet(id, base) {
              const path = id.startsWith('.')
                ? resolve(base, id)
                : require.resolve(id, { paths: [base] });
              return {
                path,
                base: dirname(path),
                content: await readFile(path, 'utf8'),
              };
            },
          },
        );
        const classes = [...html.matchAll(/class="([^"]+)"/g)].flatMap(
          ([, value]) => value.split(/\s+/),
        );
        return html.replace(
          '<!-- offline-styles -->',
          `<style>${compiler.build(classes)}</style>`,
        );
      },
    },
  };
}
