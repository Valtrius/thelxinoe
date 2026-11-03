// Loaded only when a styled track is shown: the renderer is libass compiled to WebAssembly.
import JASSUB from 'jassub';
import workerUrl from 'jassub/dist/worker/worker.js?worker&url';
import wasmUrl from 'jassub/dist/wasm/jassub-worker.wasm?url';
import modernWasmUrl from 'jassub/dist/wasm/jassub-worker-modern.wasm?url';
import fallbackFont from 'jassub/dist/default.woff2?url';

export type StyledRenderer = JASSUB;

/** Draws `script` over `video` into `canvas`, with the file's attached `fonts`. */
export function render(
  video: HTMLVideoElement,
  canvas: HTMLCanvasElement,
  script: string,
  fonts: string[],
  timeOffset: number,
) {
  return new JASSUB({
    video,
    canvas,
    subContent: script,
    fonts,
    timeOffset,
    workerUrl,
    wasmUrl,
    modernWasmUrl,
    availableFonts: { 'liberation sans': fallbackFont },
  });
}

/** `script` with the text of each of its styles scaled by `size`. */
export function scaled(script: string, size: number) {
  if (size === 1) return script;
  let styles = false,
    column = -1;
  return script.replace(/^.*$/gm, (line) => {
    const text = line.trim();
    if (text.startsWith('[')) {
      styles = /^\[v4\+? styles\]$/i.test(text);
      column = -1;
      return line;
    }
    const separator = line.indexOf(':');
    if (!styles || separator < 0) return line;
    const key = line.slice(0, separator).trim().toLowerCase();
    const fields = line.slice(separator + 1).split(',');
    if (key === 'format')
      column = fields.findIndex((f) => f.trim().toLowerCase() === 'fontsize');
    if (key !== 'style' || column < 0 || !(column in fields)) return line;
    const value = Number(fields[column]);
    if (!Number.isFinite(value)) return line;
    fields[column] = String(Math.round(value * size * 100) / 100);
    return `${line.slice(0, separator + 1)}${fields.join(',')}`;
  });
}
