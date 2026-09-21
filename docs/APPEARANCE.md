# Appearance and navigation

The web and Windows clients share the same palette, square surfaces, typography, navigation components and layout motion. Shared Svelte components live in `frontend/src/lib/ui`; color tokens live in `frontend/src/app.css`.

Light, System and Dark controls are in the Windows title bar and at the bottom of the web sidebar. Account preferences are stored by the server through `GET/PATCH /api/v1/me/appearance`: theme, sidebar expansion, card density, watched-video fading and YouTube card shortcuts. Patches change only the supplied fields. The device caches the selected theme for the sign-in screen.

Media grids use 320-pixel logical cards, 12-pixel gaps, Ctrl+wheel density control, scroll anchoring and 200 ms layout transitions. Small screens reduce the column count. Reduced-motion preferences disable movement. Sidebar icons remain in fixed slots while labels and content animate. YouTube has date groups with sticky headers and a watchlist dock; pending additions appear immediately and the URL input clears before the request completes.

Movies and shows use posters, music uses square artwork, and online streams use landscape previews. Home and child items inherit cached artwork from their show or album when they have none of their own. Artwork remains protected by session-bound grants. Live previews use public image addresses returned by the providers; provider credentials stay on the server.

The web video player appears immediately with its title and a loading indicator while the server prepares the stream. The title and playback controls overlay the video and fade after inactivity; moving the pointer or focusing a control reveals them. Pausing, loading and errors keep them visible. Quality, audio and subtitle choices open inside the player. Fullscreen includes the entire player, so its controls, settings and segment prompts remain available. Buffering feedback follows media readiness, including after a seek, and reduced-motion preferences disable the spinner animation and overlay transitions.

Settings has a separate navigation column. MPV and server connection settings appear only on Windows. Playback tools are server dependencies and have no web or desktop tool menu; the server queues verified installation when YouTube playback first needs them. Administrative settings appear only for administrators. Statistics uses actual playback history, including daily watch time and most-watched titles, with the same user/date scope as history.

The desktop updater has an empty bootstrap configuration so ordinary builds start without a release channel. Download endpoints and artifact keys are supplied only after verification of the signed product release manifest.

## Provider views

`frontend/src/lib/providers/components` contains the YouTube, Twitch and Kick views, cards, toolbars, popovers and watchlist sidebar. Their controllers handle search shortcuts, grouping, drag ordering, shortcut selection, optimistic additions and animations. `providers/api.ts` connects those contracts to the authenticated server; its motion modules re-export the shared engine so sidebar and grid animations have one owner. Library and Settings form styles are scoped away from these components.

Every page has a slim title row, followed by the provider toolbar where relevant. Global notifications live in the main sidebar and dismiss on an outside click or Escape. Settings panels remain left aligned at a maximum width of 880 pixels and use the shared Switch component for boolean settings, including watchlist options. First administrator creation asks for matching passwords; ordinary sign-in has one password field.

Provider views follow these server and client boundaries:

- Web playback uses the browser player; Windows uses MPV. Extraction and download tools run on the server, so these pages have no local tool configuration. Signed-in browser-cookie playback and its shortcut are omitted because the server supports public extraction.
- Account linking uses server application credentials and encrypted server token storage. Connection management is in **Settings → Online accounts**; application credentials are in the administrator's **Provider applications** page.
- Downloading adds a personal retention pin. A shared download cannot be deleted or cancelled while another user retains it or while it is playing. Downloads and automatic watchlist downloads require the administrator's download setting.
- Refresh requests retain server cooldowns and provider quota/backoff limits. YouTube backfill remains bounded to 150 uploads or 90 days per channel. Provider metadata refreshes asynchronously.

Named watchlists, their settings and membership are private server data. Existing saved videos migrate into Watch Later. Aggregate membership continues to protect downloads from retention cleanup. Manual reordering applies immediately and rolls back with an error if the server rejects it. Private WebSocket events update other connected sessions. Card shortcuts, feed filters, watchlist expansion/selection and browser playback volume persist with account appearance preferences and synchronize between clients of the same user. Each device remembers its last page and Settings tab for that server and user.

Windows MPV settings include executable discovery, managed versions, update policies, pinning, rollback, repair, local configuration imports, live option discovery, and uosc/thumbfast/sub-select controls. Existing Thelxinoe MPV settings migrate without deleting the original files. Player sessions lease their executable/plugin versions and a configuration snapshot; imported scripts remain intact when a managed plugin is selected. Browser clients have no MPV settings. Desktop video controls and segment prompts stay in MPV; music retains the app's queue controls. Segment prompts use Ctrl+Enter to skip.
