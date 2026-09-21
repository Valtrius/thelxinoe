CREATE TEMP TABLE migration_manager_keep AS
SELECT current.kind,
       COALESCE(
         (SELECT provision.service_id
            FROM stack_provisions provision
            JOIN manager_services managed ON managed.id=provision.service_id AND managed.kind=current.kind
           WHERE provision.kind=current.kind
           LIMIT 1),
         (SELECT candidate.id
            FROM manager_services candidate
           WHERE candidate.kind=current.kind
           ORDER BY candidate.checked_at DESC,candidate.id
           LIMIT 1)
       ) AS id
  FROM manager_services current
 GROUP BY current.kind;

DELETE FROM acquisition_requests WHERE service_id NOT IN (SELECT id FROM migration_manager_keep);
DELETE FROM manager_bindings WHERE service_id NOT IN (SELECT id FROM migration_manager_keep);
DELETE FROM manager_reconciliations WHERE service_id NOT IN (SELECT id FROM migration_manager_keep);
DELETE FROM retention_exclusions WHERE service_id NOT IN (SELECT id FROM migration_manager_keep);
DELETE FROM metadata_bindings WHERE service_id NOT IN (SELECT id FROM migration_manager_keep);
DELETE FROM manager_episodes WHERE service_id NOT IN (SELECT id FROM migration_manager_keep);
DELETE FROM manager_services WHERE id NOT IN (SELECT id FROM migration_manager_keep);
DROP TABLE migration_manager_keep;

CREATE TEMP TABLE migration_support_keep AS
SELECT current.kind,
       COALESCE(
         (SELECT provision.service_id
            FROM stack_provisions provision
            JOIN support_services managed ON managed.id=provision.service_id AND managed.kind=current.kind
           WHERE provision.kind=current.kind
           LIMIT 1),
         (SELECT candidate.id
            FROM support_services candidate
           WHERE candidate.kind=current.kind
           ORDER BY candidate.checked_at DESC,candidate.id
           LIMIT 1)
       ) AS id
  FROM support_services current
 GROUP BY current.kind;

DELETE FROM support_services WHERE id NOT IN (SELECT id FROM migration_support_keep);
DROP TABLE migration_support_keep;

CREATE UNIQUE INDEX manager_services_kind ON manager_services(kind);
CREATE UNIQUE INDEX support_services_kind ON support_services(kind);

UPDATE media_files SET ownership='unresolved' WHERE ownership='ambiguous';
