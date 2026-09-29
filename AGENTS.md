## Frontend

- Keep SQL and SQLite access in `apps/server/src/storage`; expose typed operations and send writes through the storage writer.
- Keep API and UI integration separate from Docker lifecycle ownership; only manage containers Thelxinoe owns.


## Migration requirements

- Until an official release is done, no migration shall be necessary either in DB or code.


## Testing

- Always launch the CI tests locally before commiting or pushing to make sure CI will pass. Once launched, end your turn with a user-facing notification window programmed for CI end. User will tell you when noification appears so you can check the results.
