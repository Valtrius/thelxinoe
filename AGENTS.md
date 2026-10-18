## Frontend

- Keep SQL and SQLite access in `apps/server/src/storage`; expose typed operations and send writes through the storage writer.
- Keep API and UI integration separate from Docker lifecycle ownership; only manage containers Thelxinoe owns.


## Database schema

- Released schemas are frozen in `crates/database/schemas/`. Any later change to `schema.sql` bumps `SCHEMA_VERSION`, adds its upgrade to `MIGRATIONS` and freezes the new schema; the database tests enforce this.


## Workflow

- Always design CI tests to run on GitHub Actions and account for the runner's platform, permissions, resource limits, networking, Docker behavior, and empty caches.
- Before pushing changes, always run the format `pnpm run format`.
