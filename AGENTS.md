## Frontend

- Always use Tailwind CSS for frontend styling and UI work.
- Design reusable components with focused props; add options when real use cases need them.
- Keep SQL and SQLite access in `apps/server/src/storage`; expose typed operations and send writes through the storage writer.
- Keep API and UI integration separate from Docker lifecycle ownership; only manage containers Thelxinoe owns.
- Never add bullshit feel good phrases like `Your collection, in one place.`, `A place you can call home.` or `Your media, locally.`.


## Migration requirements

- Until an official release is done, no migration shall be necessary either in DB or code.