# Thelxinoe

A self-hosted media server: local movies, shows and music, the media services that acquire them, and YouTube, Twitch and Kick, played from the web, a desktop app or Jellyfin clients.

## Language

### Surfaces

**Server**:
The Thelxinoe process that holds the catalog, users and playback; one per deployment.
_Avoid_: Backend, product, host

**Web app**:
The browser interface to a server, installable as a PWA.
_Avoid_: Frontend, site, client

**Desktop app**:
The Windows application that wraps the web app and plays video in MPV.
_Avoid_: Windows app, native app, device, Tauri app

**Jellyfin client**:
A third-party application that talks to the server through its Jellyfin-compatible API.
_Avoid_: Media client, compat client, TV

**Section**:
A top-level destination of the app: Home, Discover, Movies, Shows, Music, YouTube, Twitch, Kick, Statistics or Settings.
_Avoid_: Page, tab, route

**Shelf**:
A horizontal row of items on Home, such as Continue watching or Favorites.
_Avoid_: Row, rail, carousel

**Player**:
The video player of the web app.
_Avoid_: Video player, media player

**Music dock**:
The persistent music player docked at the bottom edge of the app.
_Avoid_: Music player, mini player, bar

**MPV player**:
The separate MPV window in which the desktop app plays video.
_Avoid_: Native player, desktop player

### Catalog

**Catalog**:
Everything the server knows about the local media in its library folders.
_Avoid_: Database, index, library (for the whole)

**Library**:
One of the three kinds of local media: Movies, Shows or Music.
_Avoid_: Domain, media type, platform

**Library folder**:
A folder of the media mount that the server scans into one library.
_Avoid_: Library root, root, root folder (that is a manager's term)

**Scan**:
Reading a library folder to bring the catalog in line with the files on disk.
_Avoid_: Index, import, refresh

**Media item**:
Any node of the catalog: a movie, show, season, episode, artist, album or track.
_Avoid_: Entry, media, catalog item, card

**Title**:
A movie or a show, the unit that has its own page and that people search, request and browse.
_Avoid_: Item, entry, media

**Show**:
A title made of seasons and episodes.
_Avoid_: Series, TV, TV show

**Specials**:
The season of a show that holds episodes outside its numbered seasons.
_Avoid_: Season 0

**Playable item**:
A media item that plays by itself: a movie, an episode or a track.
_Avoid_: Leaf, video

**Media file**:
One physical file in a library folder; it can stand for several media items, and a media item can have several.
_Avoid_: Source, media source, version

**Edition**:
A named cut of a movie, such as a director's cut, carried by one or more of its media files; the unnamed one is the Original edition.
_Avoid_: Version, cut, release

**File version**:
The exact content of a media file at a point in time; replacing the file's content makes a new one.
_Avoid_: Generation, revision, fingerprint

**Local trailer**:
A trailer file stored beside a movie in its library folder.
_Avoid_: Extra

**Available**:
Said of a media item that has at least one media file present on disk.
_Avoid_: Downloaded, present, in library

**Title status**:
What a title is doing in its manager: available, partial, downloading, queued, upcoming, missing or unmonitored.
_Avoid_: State, availability, phase

**Metadata**:
The descriptive data of a media item — names, artwork, dates, ratings — obtained from its manager.
_Avoid_: Info, details

### Watching

**Watched**:
A user's mark that they have seen a playable item; a season or show is watched when all its episodes are.
_Avoid_: Played, completed, finished, seen

**In progress**:
Said of a playable item a user started and has not yet watched.
_Avoid_: Started, resumable, partially watched

**Resume point**:
The position at which a user left an edition of a playable item.
_Avoid_: Progress, edition progress, playback position

**Continue watching**:
The shelf of movies and episodes a user has in progress.
_Avoid_: Resume, keep watching

**Next up**:
The first available episode of a show after the last one a user watched.
_Avoid_: Next episode, up next

**Up next**:
The prompt at the end of an episode that offers, or starts, the episode that follows it.
_Avoid_: Next up, autoplay

**Favorite**:
A user's mark on a media item or playlist they want to find again.
_Avoid_: Like, star, pin

**Watch Later**:
A user's mark on a title, season or episode they intend to watch; it clears once watched.
_Avoid_: Watchlist, saved, queue

**Playlist**:
A user's ordered list of tracks, optionally shared with other users.
_Avoid_: Watchlist, queue, collection

**Queue**:
The tracks lined up to play in the music dock.
_Avoid_: Playlist, up next

**Playback history**:
The record of what each user played, when, and on which device; it outlives the media.
_Avoid_: History, watch history, activity

**Statistics**:
Aggregates of watch time and completions drawn from playback.
_Avoid_: History, analytics

### Playback

**Playback session**:
One attempt by one user to play one thing: a media file, a YouTube video or a live stream.
_Avoid_: Session, play session, stream, run

**Playback mode**:
How the server delivers a playback session: direct play, remux or transcode.
_Avoid_: Delivery, method, stream type

**Direct play**:
The playback mode in which the client plays the media file as it is.
_Avoid_: Original, direct stream

**Remux**:
The playback mode in which the server repackages the media without re-encoding it.
_Avoid_: Direct stream, conversion

**Transcode**:
The playback mode in which the server re-encodes the video, the audio or both.
_Avoid_: Conversion, convert, encode

**Hardware transcoding**:
Transcoding done by the server's GPU instead of its CPU.
_Avoid_: Hardware conversion, hardware acceleration, VAAPI

**Quality**:
The user's cap on what a playback session delivers: Auto, Original, a resolution or a bitrate.
_Avoid_: Resolution, bitrate, profile

**Track** (playback):
An audio or subtitle stream of what is playing. Not a music track.
_Avoid_: Stream, language

**Burned-in subtitle**:
A subtitle rendered into the video by a transcode because the client cannot draw it.
_Avoid_: Hardsub, picture subtitle

**Skip segment**:
A marked span of a playable item that a user can skip: intro, recap, credits or preview.
_Avoid_: Segment, chapter, marker

**Episode analysis**:
The background detection of skip segments by comparing the episodes of a season.
_Avoid_: Segment analysis, intro detection, fingerprinting

**Trickplay**:
The thumbnails shown while seeking through a video.
_Avoid_: Seek preview, preview, scrubbing thumbnails

**Playback grant**:
A short-lived token that lets a player fetch the media of one playback session.
_Avoid_: Ticket, token, signed URL

### Media services

**Media service**:
One of the external applications Thelxinoe works with: Radarr, Sonarr, Lidarr, Bazarr, Prowlarr, NZBGet, Seerr or Recyclarr.
_Avoid_: Service (alone, in prose), integration, arr, app

**Manager**:
A media service that acquires and organises a library: Radarr for movies, Sonarr for shows, Lidarr for music.
_Avoid_: Acquisition manager, media manager, arr

**Support service**:
A media service that serves the managers rather than a library: Bazarr, Prowlarr, NZBGet and Seerr.
_Avoid_: Helper, companion

**Connected**:
The stage of a media service that someone else runs and Thelxinoe only validates; it is never written to.
_Avoid_: External, existing, attached, linked, registered

**Owned**:
The stage of a media service whose container Thelxinoe runs and manages.
_Avoid_: Managed, installed, adopted

**Operational**:
Said of an owned media service that finished setup and is under no operational hold, so normal work may use it.
_Avoid_: Ready, active, enabled

**Connection**:
Thelxinoe's saved record of how to reach a media service: its container, address and credentials.
_Avoid_: Registration, endpoint, integration, link

**Install**:
Creating a new media service that is owned from the start.
_Avoid_: Provision, deploy, create

**Ownership transfer**:
The guided process that turns a connected media service into an owned one by running a replacement of it.
_Avoid_: Takeover, adoption, migration, import

**Original**:
The container a media service ran in before an ownership transfer; it is stopped, never modified.
_Avoid_: Source, old container, external container

**Replacement**:
The container Thelxinoe runs in place of the original after an ownership transfer.
_Avoid_: Candidate, successor, copy

**Transfer snapshot**:
The untouched copy of the original's configuration taken by an ownership transfer.
_Avoid_: Backup, snapshot (alone)

**Transfer recovery**:
The choices offered when an ownership transfer fails after the replacement started: repair, restore the transfer snapshot, return to the original, or suspend.
_Avoid_: Rollback, retry

**Operational hold**:
A block on normal work with an owned media service while it is being set up, transferred or recovered.
_Avoid_: Lock, pause, maintenance

**Setup**:
The configuration Thelxinoe applies to a media service it has just installed or taken ownership of.
_Avoid_: Bootstrap, provisioning, wiring

**Service link**:
A configured relation between two media services, such as Prowlarr feeding Radarr or Radarr using NZBGet.
_Avoid_: Connection, service connection, wiring, binding

**Retire**:
Dropping an owned media service whose container is gone, keeping its configuration.
_Avoid_: Remove, forget, uninstall

**Remove**:
Deleting an owned media service together with its container and configuration.
_Avoid_: Retire, uninstall, delete

**Forget**:
Deleting the connection to a connected media service, leaving the service untouched.
_Avoid_: Disconnect, remove

**Managed file**:
A media file that a manager claims as its own, as opposed to an unmanaged one that no manager knows.
_Avoid_: Owned file, file ownership, bound file

**Controller**:
The companion process that performs every Docker operation on behalf of the server.
_Avoid_: Docker controller, daemon, agent

**Deployment**:
One installation of Thelxinoe as a whole: its server, controller, storage and owned media services.
_Avoid_: Stack, instance, install

**Adoption**:
The controller accepting containers recreated outside it as the same deployment.
_Avoid_: Takeover, ownership transfer

### Requests and deletion

**Request**:
A user's ask for a title, album or artist to be acquired, which an administrator approves or declines.
_Avoid_: Acquisition, order, wish

**Declined**:
The outcome of a request an administrator refused.
_Avoid_: Denied, rejected

**Monitored**:
Said of a title its manager keeps looking for and upgrading.
_Avoid_: Tracked, watched, followed

**Media operation**:
A recorded destructive change to a media item's files: delete, unmonitor or monitor.
_Avoid_: Job, action, task

**Keep**:
An administrator's mark that protects a media item and everything beneath it from deletion.
_Avoid_: Protect, pin, lock, favorite

**Auto-delete**:
The policy that deletes watched movies, shows and downloaded videos after a delay, or when storage runs out.
_Avoid_: Retention, cleanup, pruning

**Scheduled deletion**:
A media item or download that auto-delete has selected and will delete when its delay ends.
_Avoid_: Retention candidate, candidate, pending deletion

**Exclusion**:
A manager's record that a deleted title must not be acquired again.
_Avoid_: Blocklist, ignore

### Background work

**Job**:
A unit of background work the server queues and retries, such as a scan or a service install.
_Avoid_: Task, operation, worker

**Event**:
A notification of a change that the server pushes to connected apps.
_Avoid_: Message, notification, update

**Attention item**:
A condition the server raises for a user to act on, shown as a dot in the navigation until dismissed.
_Avoid_: Notification, alert, warning, marker

**Audit log**:
The record of administrative and security actions and who performed them.
_Avoid_: Activity, history, log

### Updates and backups

**Release**:
A signed, published version of Thelxinoe: its server, controller and desktop app.
_Avoid_: Build, version, product

**Server update**:
Replacing the running server and controller with a newer release.
_Avoid_: Product update, upgrade, self-update

**Service update**:
Replacing an owned media service's container with a newer image.
_Avoid_: Container update, upgrade

**Maintenance window**:
The daily hours in which automatic updates and backups may run.
_Avoid_: Update window, schedule

**Rollback**:
Returning to the state before an update that failed.
_Avoid_: Recovery, restore, downgrade

**Backup**:
An encrypted archive of the server's state and its owned media services' configuration.
_Avoid_: Snapshot, export, dump

**Server tool**:
An external binary the server installs and updates for itself: yt-dlp, Deno, Streamlink or FFmpeg.
_Avoid_: Tool (alone), bundled binary, dependency

**Desktop tool**:
An external program the desktop app installs for playback: MPV and its plugins.
_Avoid_: Tool (alone), plugin

### Online providers

**Provider**:
One of the online video platforms Thelxinoe follows: YouTube, Twitch or Kick.
_Avoid_: Platform, online, integration, source

**Provider application**:
The credentials an administrator registers with a provider so the server may call its API.
_Avoid_: Client, API key, integration

**Online account**:
A user's own account at a provider, connected to their Thelxinoe user.
_Avoid_: Linked account, provider account, connection

**Connect**:
Authorising the server to read a user's online account. Its opposite is Disconnect.
_Avoid_: Link, sign in, log in

**Sync**:
Fetching a user's subscriptions, channels and videos from a provider.
_Avoid_: Refresh, import, poll

**Quota**:
The daily budget of YouTube API calls shared by all users of a server.
_Avoid_: Rate limit, budget

**Channel**:
A provider's publisher of videos or live streams.
_Avoid_: Streamer, creator, user

**Subscription**:
A YouTube channel a user subscribes to on YouTube.
_Avoid_: Follow, tracked channel

**Followed channel**:
A Twitch channel a user follows on Twitch.
_Avoid_: Subscription, tracked channel

**Tracked channel**:
A Kick channel a user added in Thelxinoe; Kick has no account to follow from.
_Avoid_: Subscription, followed channel, saved channel

**Video**:
A recorded YouTube upload.
_Avoid_: Upload, VOD, media item

**Short**:
A YouTube video published in the Shorts format.
_Avoid_: Clip

**Live stream**:
A broadcast that is happening now on a provider.
_Avoid_: Stream, live, broadcast

**Live replay**:
The recording of a YouTube live stream that has ended.
_Avoid_: VOD, replay, past broadcast

**Upcoming**:
Said of a YouTube live stream that is scheduled and has not started.
_Avoid_: Scheduled, premiere

**Feed**:
A user's YouTube videos from their subscriptions, filtered and grouped by date.
_Avoid_: Timeline, home, browse

**Watchlist**:
A user's named list of YouTube videos.
_Avoid_: Playlist, Watch Later (that is the local mark), saved videos

**Download**:
The server's stored copy of a YouTube video, shared by every user who wants it.
_Avoid_: Offline copy, saved video, cache

**Download request**:
One user's ask that a video be downloaded; a download lives while any user still wants it.
_Avoid_: Request (alone), queue entry

**Pin**:
A user's mark that keeps a YouTube video and its download from being removed.
_Avoid_: Keep, favorite, save

**Unavailable** (video):
Said of a video that YouTube no longer serves.
_Avoid_: Deleted, private, missing

**Public extraction**:
Resolving a provider's public media addresses without any user's credentials.
_Avoid_: Scraping, resolving, extraction (alone)

### Identity

**User**:
A person who can sign in to a server.
_Avoid_: Account, person, member, viewer, profile

**Administrator**:
A user with the role that may manage the server, its media services and other users.
_Avoid_: Admin, owner, superuser

**Role**:
What a user is allowed to do: administrator or user.
_Avoid_: Permission, group, access level

**Session**:
One sign-in of a user from one browser, desktop app or Jellyfin client.
_Avoid_: Login, device, token, playback session

**Sign-in method**:
A way a user proves who they are: password, passkey, authenticator app or identity provider.
_Avoid_: Auth method, credential, factor

**Authenticator app**:
The sign-in method that asks for a time-based code after the password.
_Avoid_: TOTP, 2FA, two-step sign-in

**Identity provider**:
The external single sign-on service a server can delegate sign-in to.
_Avoid_: OIDC, SSO, provider (alone)

**Verification**:
Proving one's identity again, within a session, before a sensitive action.
_Avoid_: Re-authentication, confirmation, validation

**Remembered device**:
A browser allowed to skip the authenticator app code at sign-in.
_Avoid_: Trusted device, known browser

**Known browser**:
A browser, desktop app or Jellyfin client that has signed in to a user before, so failed attempts from elsewhere cannot lock it out.
_Avoid_: Remembered device, trusted device

**Client password**:
A generated password that lets a Jellyfin client sign in as a user, and nothing else.
_Avoid_: App password, API key, token

**Recovery link**:
A one-time link an administrator issues so a user who lost their sign-in methods can set new ones.
_Avoid_: Recovery code, reset link

**Desktop approval**:
Signing in the desktop app by approving a pairing code from an already signed-in browser.
_Avoid_: Desktop request, pairing, device login

### Jellyfin

**Jellyfin-compatible API**:
The part of the server that answers as a Jellyfin server would, for Jellyfin clients.
_Avoid_: Compat, compatibility layer, Jellyfin adapter

**Quick Connect**:
Signing in a Jellyfin client by typing the code it displays into a signed-in web app.
_Avoid_: TV code, device pairing, desktop approval

**Jellyfin import**:
Copying watched marks, resume points and favorites from an existing Jellyfin server into Thelxinoe.
_Avoid_: Migration, Jellyfin sync

**Jellyfin account**:
A user of the Jellyfin server being imported from, mapped to a Thelxinoe user.
_Avoid_: User, person

### Development

**Mock preview**:
The web app running against disposable sample data, with no server.
_Avoid_: Demo, mock server, preview (alone)

**Online profile**:
The saved development state that keeps real provider connections and LAN access between runs.
_Avoid_: Online (alone), dev server, online account

**Fresh profile**:
A new, empty development state created for one run.
_Avoid_: Clean install, reset
