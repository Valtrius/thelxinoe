// These adapters are compiled only into .local/mock, never frontend/dist.
export function mockUiPlugin() {
  return {
    name: 'mock-ui-boundaries',
    enforce: 'pre',
    transform(source, id) {
      const path = id.replaceAll('\\', '/');
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
