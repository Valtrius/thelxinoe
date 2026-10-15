// These adapters are compiled only into .local/mock, never frontend/dist.
export function mockUiPlugin() {
  return {
    name: 'mock-ui-boundaries',
    enforce: 'pre',
    transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/lib/ServicesSettings.svelte'))
        return source.replace(
          '</script>',
          `
  onMount(() => {
    const inspectMockTransfer = (event: Event) => {
      const transfer = (event as CustomEvent).detail;
      ownershipModal = { kind: transfer.kind, serviceId: transfer.service_id, transferId: transfer.id };
    };
    window.addEventListener('thelxinoe-mock-transfer', inspectMockTransfer);
    return () => window.removeEventListener('thelxinoe-mock-transfer', inspectMockTransfer);
  });
</script>`,
        );
      if (path.endsWith('/src/main.ts'))
        return `${source}\nimport ServiceScenarios from './lib/dev/ServiceScenarios.svelte';
const mockControlsTarget = document.body.appendChild(document.createElement('div'));
mockControlsTarget.dataset.mockControls = '';
mount(ServiceScenarios, { target: mockControlsTarget });
// Modal dialogs make the rest of the document inert. Keep the controls usable
// inside the active dialog, including after it closes or is removed.
new MutationObserver(() => {
  const dialogs = [...document.querySelectorAll('dialog[open]')];
  const parent = dialogs.at(-1) ?? document.body;
  if (mockControlsTarget.parentElement !== parent) parent.appendChild(mockControlsTarget);
}).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['open'] });\n`;
      if (path.endsWith('/lib/seerr/types.ts'))
        return source.replace(
          '  return path &&',
          "  if (path?.startsWith('/mock-art/')) return path;\n  return path &&",
        );
      if (path.endsWith('/lib/authentication.ts'))
        return source
          .replace(
            /^export async function passkeySignIn[\s\S]*?(?=^export async function registerPasskey)/m,
            "export async function passkeySignIn(verify = false): Promise<LoginResponse> {\n  return api('/auth/passkey/finish', 'POST', { verify });\n}\n",
          )
          .replace(
            /^export async function registerPasskey[\s\S]*?(?=^export async function oidcSignIn)/m,
            "export async function registerPasskey(name = 'Passkey') {\n  return api('/me/auth/passkeys/register/finish', 'POST', { name });\n}\n",
          );
      if (path.endsWith('/lib/providers/api.ts'))
        return source.replace(
          '    const target = new URL(url);',
          "    const target = new URL(url);\n    if (target.origin === location.origin && target.pathname === '/__mock/connect/youtube') {\n      location.assign(target.href);\n      return 'youtube';\n    }",
        );
    },
  };
}
