# ImmichSync Implementation Plan

## Phase 1: Core Engine (MVP) ✓

The minimum viable product — watch a folder, upload new files, show status in the tray.

### 1.1 Project Skeleton ✓
- [x] Initialize Cargo project with workspace structure
- [x] Set up `Cargo.toml` with all dependencies
- [x] Create `build.rs` for Windows resource compilation (icon, manifest)
- [x] Create `assets/app.manifest` (DPI awareness, no admin required)
- [x] Create placeholder icons (generated 16×16 RGBA bitmaps per state)
- [x] Add `.gitignore`, `LICENSE` (GPL-3.0-only)
- [x] Verify project compiles on Windows

### 1.2 Configuration (`src/config.rs`) ✓
- [x] Define `Config` struct with serde derive
- [x] TOML loading from `%APPDATA%\bees-roadhouse\immichsync\config.toml`
- [x] TOML saving with atomic write (write to temp, rename)
- [x] Default config generation on first run
- [x] Config directory creation if missing

### 1.3 State Database (`src/db.rs`) ✓
- [x] Initialize SQLite at `%APPDATA%\bees-roadhouse\immichsync\state.db` with WAL mode
- [x] Schema creation (watched_folders, uploaded_files, upload_queue, config tables)
- [x] Create indexes on `uploaded_files.file_hash`, `uploaded_files.file_path`, `upload_queue.status`
- [x] Schema migration system via `config.schema_version`
- [x] CRUD operations for each table
- [x] Dedup lookup by hash

### 1.4 Immich API Client (`src/api/`) ✓
- [x] `server.rs` — ping endpoint, server info, health check
- [x] `auth.rs` — API key validation, connection test
- [x] `assets.rs` — multipart file upload (`POST /api/assets`)
- [x] `assets.rs` — bulk upload check (`POST /api/assets/bulk-upload-check`)
- [x] `albums.rs` — create album, add assets to album
- [x] Error types for API failures (auth, network, server errors)
- [x] Configurable timeout and base URL

### 1.5 Watch Engine — Local Folders (`src/watch/`) ✓
- [x] `folder.rs` — `notify::RecommendedWatcher` with recursive watching
- [x] `filter.rs` — file extension filtering (images + videos)
- [x] `filter.rs` — exclusion list (Thumbs.db, desktop.ini, etc.)
- [x] `filter.rs` — minimum file size check
- [x] `mod.rs` — watch engine orchestrator, manages active watchers
- [x] Debouncing via `notify-debouncer-full`
- [x] Write completion detection (size stability + read lock check)

### 1.6 Upload Pipeline (`src/upload/`) ✓
- [x] `hasher.rs` — SHA-1 hashing with file read
- [x] `queue.rs` — persistent queue backed by SQLite (transactional state transitions)
- [x] `worker.rs` — async upload workers (tokio tasks)
- [x] `metadata.rs` — EXIF date extraction, file timestamp fallback
- [x] `mod.rs` — pipeline orchestrator: hash -> dedup check -> server bulk check -> queue -> upload -> record
- [x] Configurable concurrency (default 2)
- [x] Exponential backoff retry (1s, 2s, 4s... up to 32s, max 5 retries)
- [x] Device asset ID generation (`{path_hash}-{filename}`)
- [x] Device ID generation (`immichsync-{machine_name}`)

### 1.7 System Tray (`src/ui/tray.rs`) ✓
- [x] Tray icon initialization with `tray-icon` + `winit`
- [x] Icon state switching (idle, syncing, error, offline)
- [x] Right-click context menu (pause, settings, quit)
- [x] Upload progress in menu (X/Y files)
- [x] Server connection status in menu

### 1.8 Platform Basics (`src/platform/`) ✓
- [x] `single_instance.rs` — named mutex (`Global\ImmichSync`)
- [x] `known_folders.rs` — `SHGetKnownFolderPath(FOLDERID_Pictures)` wrapper
- [x] Auto-detect Pictures folder on first run

### 1.9 Application Lifecycle (`src/main.rs`, `src/app.rs`) ✓
- [x] Single-instance check on startup
- [x] Load config, open database
- [x] Start watch engine on configured folders
- [x] Start upload pipeline
- [x] Start tray icon
- [x] Graceful shutdown on quit (flush queue, close watchers)

---

## Phase 2: Full Watch Support ✓

### 2.1 Network Share Support (`src/watch/network.rs`) ✓
- [x] Detect UNC paths and mapped drives
- [x] Health check: create temp file, wait for native event
- [x] Auto-fallback to `PollWatcher` if no event within 5s
- [x] Configurable poll interval per folder
- [x] Log watcher mode per path

### 2.2 Removable Device Monitoring (`src/watch/device.rs`) ✓
- [x] Hidden message-only window for `WM_DEVICECHANGE`
- [x] Handle `DBT_DEVICEARRIVAL`: resolve drive letter, enumerate volumes
- [x] Handle `DBT_DEVICEREMOVECOMPLETE`: tear down watcher, flush queue
- [x] DCIM folder auto-detection (cameras)
- [x] Configurable: auto-watch / ask / ignore on insert

### 2.3 Platform — Drive Enumeration (`src/platform/drives.rs`) ✓
- [x] List available drives with type (fixed, removable, network)
- [x] Detect drive letter from device notification
- [x] Volume label reading for display

### 2.4 Watch Engine Enhancements
- [ ] Per-folder include/exclude glob patterns (UI support)
- [x] Per-folder album mode configuration
- [x] Dynamic add/remove watchers at runtime (for settings changes)
- [ ] Watcher health monitoring and auto-restart

---

## Phase 3: Settings & Polish ✓

### 3.1 Settings Window (`src/ui/settings.rs`) ✓
- [x] Connection tab: server URL, API key (masked input), test connection, device name
- [x] Watch Folders tab: list with toggles, add/remove folders, per-folder config
- [x] Devices tab: auto-watch toggle, DCIM toggle, insert action dropdown
- [x] Upload tab: concurrency slider, bandwidth limit, upload order, retry count, file types
- [x] Advanced tab: poll interval, log level, autostart, minimize-to-tray, db path, export/import

### 3.2 First-Run Setup (`src/ui/first_run.rs`) ✓
- [x] Welcome screen
- [x] Server URL + API key entry with test
- [x] Pictures folder confirmation
- [x] Start-with-Windows option
- [x] Done — start syncing

### 3.3 Album Auto-Creation ✓
- [x] Create album from parent folder name
- [x] Create album from subfolder name
- [x] Create album from date (YYYY-MM)
- [x] Add to fixed user-selected album
- [x] Album caching to avoid redundant API calls

### 3.4 Notifications (`src/ui/notifications.rs`) ✓
- [x] Windows toast notifications
- [x] Notify on batch upload complete
- [x] Notify on error (server unreachable, auth failed)
- [x] Configurable: enable/disable per notification type

### 3.5 Upload Log Viewer ✓
- [x] Simple scrollable list of recent uploads
- [x] Show: filename, timestamp, status, error message
- [x] Filter by status (completed, failed)

### 3.6 Bandwidth Throttling ✓
- [x] Token bucket rate limiter on upload stream
- [x] Configurable KB/s limit (0 = unlimited)
- [x] Dynamic adjustment via settings without restart

### 3.7 Auto-Start (`src/platform/autostart.rs`) ✓
- [x] Registry key at `HKCU\...\Run`
- [x] Add/remove on settings toggle
- [x] Detect current state on settings load

### 3.8 API Key Encryption ✓
- [x] AES-256-GCM encryption with DPAPI-derived key
- [x] Encrypt on save, decrypt on load
- [x] Fallback to plaintext with warning if DPAPI unavailable

---

## API Key Permissions Required

ImmichSync requires an API key with the following permissions. On Immich server
versions prior to v1.138.0 the key must use `all` permissions. On newer servers
create a key with at least these scopes:

| Permission | Used By | Endpoint |
|---|---|---|
| `asset.upload` | Upload photos/videos + bulk dedup check | `POST /api/assets`, `POST /api/assets/bulk-upload-check` |
| `album.create` | Auto-create albums | `POST /api/albums` |
| `album.read` | List albums to find existing ones | `GET /api/albums` |
| `albumAsset.create` | Add assets to albums | `PUT /api/albums/{id}/assets` |
| `user.read` | Validate API key / get current user | `GET /api/users/me` |
| `server.about` | Server version and details | `GET /api/server/about` |

Note: `GET /api/server/ping` requires no auth (public endpoint) and is used for
connectivity checks. The old `/api/server/info` endpoint no longer exists; use
`/api/server/about` (requires `server.about`) or `/api/server/version` (public).

When configuring an API key in the Immich web UI (User Settings > API Keys),
grant these specific permissions or use `all` for simplicity.

**Security note:** Immich versions < 2.4.1 and certain 2.5.x releases had a
privilege escalation bug (GHSA-237r-x578-h5mv) where API keys could grant
themselves additional permissions. Users should ensure they're on a patched
version.

---

## Future: OAuth / OIDC Authentication

Immich supports three auth methods: API key (`x-api-key` header), bearer token
(`Authorization: Bearer <JWT>`), and session cookie. Currently ImmichSync only
supports API key authentication.

**Important:** Immich's OIDC support is for user login only — it is NOT an OAuth
provider for third-party apps. There is no client credentials flow or way for
external apps to obtain OAuth tokens. Users log in via providers like Authentik,
Authelia, or Keycloak using the Authorization Code flow, but Immich does not
expose API scopes through OAuth.

A future release could add browser-based login to obtain a session token:

- **Login flow:** Open system browser to the Immich login page. User
  authenticates (with password or OIDC redirect). Capture the session JWT from
  the response (requires a localhost callback server or deep-link handler).
- **Token storage:** Encrypt JWT via DPAPI (same envelope encryption as API
  keys).
- **Token refresh:** Immich JWTs have expiration; would need periodic re-auth.
- **Fallback:** Keep API key auth as the primary and recommended method — it's
  simpler, more reliable, and doesn't require OIDC server configuration.
- **UI:** Add "Login with Browser" button to Connection tab alongside the API
  key field. If session is active, show "Logged in as {email}" with a logout
  button.

Reference: [Immich OAuth docs](https://docs.immich.app/administration/oauth/),
[Immich API docs](https://api.immich.app/getting-started),
[Immich OpenAPI spec](https://github.com/immich-app/immich/blob/main/open-api/immich-openapi-specs.json)

---

## Phase 4: Ship It

### 4.1 Self-Install ✓
- [x] Single exe copies itself to `%APPDATA%\bees-roadhouse\immichsync\`
- [x] Install dialog with desktop/start menu shortcut creation
- [x] Portable mode option (skip install, run from current location)
- [x] Update dialog when newer version is run over existing install
- [x] Version tracking via `version.txt`
- [x] Legacy data migration from old `%APPDATA%\ImmichSync\` path

### 4.2 Auto-Update ✓
- [x] Check GitHub Releases API for newer version
- [x] Notify user via tray notification
- [x] Download with progress bar
- [x] Apply update via rename dance (running→.old, new→running)
- [x] Relaunch after update
- [x] Cleanup leftover `.exe.old` on next startup

### 4.3 Build & Release Pipeline
- [ ] GitHub Actions CI (build, test, clippy, fmt)
- [ ] Release workflow: build -> create GitHub Release
- [x] Manual release via `gh release create`

### 4.4 Code Signing
- [ ] Obtain code signing certificate
- [ ] Sign executable
- [ ] SmartScreen reputation building

### 4.5 Documentation & Marketing
- [ ] Screenshots and demo GIF for README
- [ ] Landing page (simple GitHub Pages or standalone)
- [ ] Post to Immich community (GitHub Discussions, Reddit r/selfhosted)
- [x] GitHub Sponsors / Buy Me a Coffee / Ko-fi links in README

---

## Remaining Work

Tracked in the [issue tracker](https://github.com/bees-roadhouse/immichsync/issues?q=is%3Aopen+label%3Aenhancement) under the `enhancement` label.

---

## Milestones

| Milestone | Description | Status |
|-----------|-------------|--------|
| **M1** | Compiles and runs | ✓ |
| **M2** | Watches and uploads | ✓ |
| **M3** | Reliable sync | ✓ |
| **M4** | Full device support | ✓ |
| **M5** | Settings UI | ✓ |
| **M6** | Release-ready | ✓ v0.1.0 released |
