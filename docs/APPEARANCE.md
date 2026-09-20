# Appearance and navigation

The web and Windows clients share the YouTwitch palette, square surfaces, typography, navigation components and layout motion. Shared Svelte components live in `frontend/src/lib/ui`; color tokens live in `frontend/src/app.css`.

Light, System and Dark controls are in the Windows title bar and at the bottom of the web sidebar. Account preferences are stored by the server through `GET/PATCH /api/v1/me/appearance`: theme, sidebar expansion, card density, watched-video fading and thumbnail fit. Patches change only the supplied fields. The device caches the selected theme for the sign-in screen.

Media grids use YouTwitch's 320-pixel logical cards, 12-pixel gaps, Ctrl+wheel density control, scroll anchoring and 200 ms layout transitions. Small screens reduce the column count. Reduced-motion preferences disable movement. Sidebar icons remain in fixed slots while labels and content animate. YouTube has date groups with sticky headers and a watchlist dock; pending additions appear immediately and the URL input clears before the request completes.

Movies and shows use posters, music uses square artwork, and online streams use landscape previews. Home and child items inherit cached artwork from their show or album when they have none of their own. Artwork remains protected by session-bound grants. Live previews use public image addresses returned by the providers; provider credentials stay on the server.

Settings has a separate navigation column. MPV and server connection settings appear only on Windows. Playback tools are server dependencies and have no web or desktop tool menu; the server queues verified installation when YouTube playback first needs them. Administrative settings appear only for administrators. Statistics uses actual playback history, including daily watch time and most-watched titles, with the same user/date scope as history.

The desktop updater has an empty bootstrap configuration so ordinary builds start without a release channel. Download endpoints and artifact keys are supplied only after verification of the signed product release manifest.
