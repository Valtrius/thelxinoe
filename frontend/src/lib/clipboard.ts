/**
 * Copies text, falling back to a selection copy where the Clipboard API is unavailable
 * (plain-HTTP LAN addresses). Inside a modal dialog pass the dialog as `container`:
 * everything outside it is inert and cannot be selected.
 */
export async function copyText(value: string, container?: Element | null) {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      // Fall through to the selection-based copy.
    }
  }
  const input = document.createElement('textarea');
  input.value = value;
  input.setAttribute('readonly', '');
  input.className = 'fixed -left-[9999px]';
  (container ?? document.body).appendChild(input);
  let copied: boolean;
  try {
    input.select();
    copied = document.execCommand('copy');
  } finally {
    input.remove();
  }
  if (!copied) throw new Error('Copy failed');
}
