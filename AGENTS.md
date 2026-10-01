## Frontend

- Keep SQL and SQLite access in `apps/server/src/storage`; expose typed operations and send writes through the storage writer.
- Keep API and UI integration separate from Docker lifecycle ownership; only manage containers Thelxinoe owns.


## Migration requirements

- Until an official release is done, no migration shall be necessary either in DB or code.


## Workflow

- Always design CI tests to run on GitHub Actions and account for the runner's platform, permissions, resource limits, networking, Docker behavior, and empty caches.
- Before pushing changes, always run the format `pnpm run format` and local CI `pnpm run ci:local` to make sure CI will pass. Once launched, end your turn as the `pnpm run ci:local` has a end notification built-in. User will tell you when noification appears so you can check the results.
