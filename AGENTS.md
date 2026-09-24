## Frontend

- Always use Tailwind CSS for frontend styling and UI work.
- Design reusable components with focused props; add options when real use cases need them.
- Keep SQL and SQLite access in `apps/server/src/storage`; expose typed operations and send writes through the storage writer.
- Keep API and UI integration separate from Docker lifecycle ownership; only manage containers Thelxinoe owns.
- Never add bullshit feel good phrases like `Your collection, in one place.`, `A place you can call home.` or `Your media, locally.`.


## Migration requirements

- Until an official release is done, no migration shall be necessary either in DB or code.


## Testing

- Never write unit tests after you write code.
- Highly prefer E2E tests as the sole testing mechanism. Use them to verify complex features work. At the end of E2E tests, produce a verifiable and repeatable artifact.
- If you must test a system in isolation, first write down all the ways it could fail, then write the code.
- Keep an isolated test only when it catches a concrete bug that existing E2E coverage misses. Document that failure and the coverage gap before implementation. Do not duplicate E2E assertions or test constants, field copying, mock call sequences, or implementation structure.


## Workflow

- Use conventional commits
