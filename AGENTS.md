## Frontend

- Keep SQL and SQLite access in `apps/server/src/storage`; expose typed operations and send writes through the storage writer.
- Keep API and UI integration separate from Docker lifecycle ownership; only manage containers Thelxinoe owns.


## Migration requirements

- Until an official release is done, no migration shall be necessary either in DB or code.


## Workflow

- Before pushing changes, always run the format `npm run format` and local CI `npm run ci:local` to make sure CI will pass. Once launched, end your turn as the `npm run ci:local` has a end notification built-in. User will tell you when noification appears so you can check the results.
