// Presentation for audit action keys. Categories follow the key's first
// segment so dynamic families such as `stack.*` or `media.*.*` stay grouped.
export const auditCategories = [
  'Security',
  'Users',
  'Server',
  'Updates',
  'Services',
  'Requests',
  'Library',
  'Online',
  'Retention',
  'Other',
] as const;
export type AuditCategory = (typeof auditCategories)[number];

const categoryByPrefix: Record<string, AuditCategory> = {
  auth: 'Security',
  session: 'Security',
  device: 'Security',
  user: 'Users',
  setup: 'Server',
  settings: 'Server',
  backup: 'Server',
  diagnostics: 'Server',
  tools: 'Server',
  product: 'Updates',
  service: 'Services',
  manager: 'Services',
  stack: 'Services',
  support: 'Services',
  recyclarr: 'Services',
  request: 'Requests',
  library: 'Library',
  metadata: 'Library',
  episode: 'Library',
  segments: 'Library',
  media: 'Library',
  playlist: 'Library',
  online: 'Online',
  retention: 'Retention',
  auto_delete: 'Retention',
};

const labels: Record<string, string> = {
  setup: 'Completed first-run setup',
  'settings.update': 'Updated server settings',
  'backup.create': 'Created a backup',
  'backup.restore': 'Restored a backup',
  'diagnostics.export': 'Exported diagnostics',
  'tools.settings': 'Updated tool settings',
  'tools.activated': 'Activated tools',
  'tools.recovered': 'Recovered a tool',
  'user.create': 'Created a user',
  'user.update': 'Updated a user',
  'user.delete': 'Deleted a user',
  'user.password': 'Changed a password',
  'auth.recovery': 'Issued a recovery code',
  'auth.client-password.created': 'Created a client password',
  'auth.client-password.revoked-all': 'Revoked all client passwords',
  'auth.desktop.approved': 'Approved a desktop sign-in',
  'auth.method.removed': 'Removed a sign-in method',
  'auth.oidc.configured': 'Configured single sign-on',
  'auth.oidc.linked': 'Linked a single sign-on account',
  'auth.passkey.added': 'Added a passkey',
  'auth.totp.enabled': 'Enabled an authenticator app',
  'session.revoke': 'Revoked a session',
  'device.quick-connect': 'Approved a Quick Connect device',
  'product.update.activate': 'Activated a server update',
  'product.update.install': 'Installed a server update',
  'product.update.policy': 'Changed the server update policy',
  'product.update.preflight': 'Checked a server update',
  'service.connection.edit': 'Edited a service connection',
  'service.native-url': 'Set a service URL',
  'service.update.policy': 'Changed a service update policy',
  'service.update.preflight': 'Checked a service update',
  'service.container.follow': 'Followed a recreated container',
  'manager.register': 'Registered a service',
  'manager.defaults': 'Updated service defaults',
  'manager.retention.permission': 'Changed service deletion access',
  'stack.install': 'Installed a service',
  'stack.adopt': 'Adopted a service',
  'stack.release': 'Released a service',
  'stack.remove': 'Removed a service',
  'stack.retire': 'Retired a service',
  'stack.restore-original': 'Restored original service settings',
  'support.register': 'Registered a support service',
  'recyclarr.sync': 'Started a Recyclarr sync',
  'recyclarr.configuration': 'Updated Recyclarr configuration',
  'request.grab': 'Grabbed a release',
  'request.monitor': 'Changed monitoring',
  'request.auto_approve': 'Changed request auto-approval',
  'library.add': 'Added a library',
  'metadata.override': 'Edited metadata',
  'metadata.refresh': 'Refreshed metadata',
  'episode.map': 'Mapped an episode',
  'segments.edit': 'Edited media segments',
  'media.keep': 'Changed deletion protection',
  'playlist.delete': 'Deleted a playlist',
  'online.configure': 'Configured an online provider',
  'online.enable': 'Enabled an online provider',
  'online.disable': 'Disabled an online provider',
  'online.disconnect': 'Disconnected an online account',
  'online.delete_data': 'Deleted online data',
  'online.delete-data': 'Deleted online data',
  'online.downloads.configure': 'Configured video downloads',
  'retention.policy': 'Updated an auto-delete policy',
  'retention.keep': 'Kept from auto-delete',
  'auto_delete.keep': 'Kept from auto-delete',
  'auto_delete.blocked': 'Auto-delete needs review',
  'auto_delete.watched': 'Auto-deleted after watching',
  'auto_delete.storage_limit': 'Auto-deleted for storage',
};

export function auditCategory(action: string): AuditCategory {
  return categoryByPrefix[action.split('.')[0]] ?? 'Other';
}

/** A readable label; unknown keys are spelled out from their segments. */
export function auditLabel(action: string): string {
  const words = action.replace(/[._-]+/g, ' ').trim();
  return labels[action] ?? words.charAt(0).toUpperCase() + words.slice(1);
}

export type AuditActionGroup = {
  category: AuditCategory;
  options: { label: string; value: string }[];
};

/** Filter options per category; keys sharing a label become one option. */
export function auditActionGroups(
  actions: string[],
  only: AuditCategory | '' = '',
): AuditActionGroup[] {
  return auditCategories
    .filter((category) => !only || category === only)
    .map((category) => {
      const byLabel = new Map<string, string[]>();
      for (const action of actions) {
        if (auditCategory(action) !== category) continue;
        const label = auditLabel(action);
        byLabel.set(label, [...(byLabel.get(label) ?? []), action]);
      }
      return {
        category,
        options: [...byLabel]
          .sort(([a], [b]) => a.localeCompare(b))
          .map(([label, keys]) => ({ label, value: keys.join(',') })),
      };
    })
    .filter((group) => group.options.length > 0);
}

export type AuditDay<T> = { key: string; label: string; rows: T[] };

/** Groups newest-first rows by calendar day in the viewer's timezone. */
export function auditDays<T extends { created_at?: number }>(
  rows: T[],
  timeZone: string,
  now = Date.now(),
): AuditDay<T>[] {
  const dayKey = new Intl.DateTimeFormat('en-CA', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  });
  const today = dayKey.format(now);
  const [year, month, day] = today.split('-').map(Number);
  const yesterday = new Date(Date.UTC(year, month - 1, day - 1))
    .toISOString()
    .slice(0, 10);
  const days = new Map<string, AuditDay<T>>();
  for (const row of rows) {
    const at = (row.created_at ?? 0) * 1000;
    const key = dayKey.format(at);
    const existing = days.get(key);
    if (existing) {
      existing.rows.push(row);
      continue;
    }
    const date = new Intl.DateTimeFormat('en', {
      timeZone,
      weekday: 'long',
      month: 'long',
      day: 'numeric',
      year: key.slice(0, 4) === today.slice(0, 4) ? undefined : 'numeric',
    }).format(at);
    const label =
      key === today
        ? `Today · ${date}`
        : key === yesterday
          ? `Yesterday · ${date}`
          : date;
    days.set(key, { key, label, rows: [row] });
  }
  return [...days.values()];
}
