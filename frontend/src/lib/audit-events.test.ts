import { describe, expect, it } from 'vitest';
import {
  auditActionGroups,
  auditCategory,
  auditDays,
  auditLabel,
} from './audit-events';

describe('audit event presentation', () => {
  it('labels known actions and spells out unknown ones', () => {
    expect(auditLabel('auth.passkey.added')).toBe('Added a passkey');
    expect(auditLabel('media.delete.complete')).toBe('Media delete complete');
    expect(auditLabel('stack.restore-original')).toBe(
      'Restored original service settings',
    );
  });

  it('groups whole action families by their first segment', () => {
    expect(auditCategory('session.revoke')).toBe('Security');
    expect(auditCategory('service.update.apply')).toBe('Services');
    expect(auditCategory('auto_delete.watched')).toBe('Retention');
    expect(auditCategory('unknown.thing')).toBe('Other');
  });

  it('offers one filter option per label within each category', () => {
    const actions = [
      'online.delete-data',
      'online.delete_data',
      'session.revoke',
      'auth.passkey.added',
    ];
    expect(auditActionGroups(actions)).toEqual([
      {
        category: 'Security',
        options: [
          { label: 'Added a passkey', value: 'auth.passkey.added' },
          { label: 'Revoked a session', value: 'session.revoke' },
        ],
      },
      {
        category: 'Online',
        options: [
          {
            label: 'Deleted online data',
            value: 'online.delete-data,online.delete_data',
          },
        ],
      },
    ]);
    expect(auditActionGroups(actions, 'Online')).toHaveLength(1);
  });

  it('groups rows by day in the viewer timezone', () => {
    // 2026-10-11 12:00 UTC is a Sunday afternoon in Paris.
    const now = Date.UTC(2026, 9, 11, 12);
    const at = (iso: string) => ({ created_at: Date.parse(iso) / 1000 });
    const rows = [
      at('2026-10-11T08:00:00Z'),
      at('2026-10-10T23:30:00Z'), // already Sunday in Paris
      at('2026-10-10T21:00:00Z'),
      at('2025-12-31T12:00:00Z'),
    ];
    const days = auditDays(rows, 'Europe/Paris', now);
    expect(days.map((day) => [day.label, day.rows.length])).toEqual([
      ['Today · Sunday, October 11', 2],
      ['Yesterday · Saturday, October 10', 1],
      ['Wednesday, December 31, 2025', 1],
    ]);
  });
});
