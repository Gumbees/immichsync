# ImmichSync Implementation Plan

## Phase 1: Core Engine (MVP)

The minimum viable product — watch a folder, upload new files, show status in the tray.

### 1.1 Project Skeleton
- [ ] Initialize Cargo project with workspace structure
- [ ] Set up `Cargo.toml` with all dependencies
- [ ] Create `build.rs` for Windows resource compilation (icon, manifest)
- [ ] Create `assets/app.manifest` (DPI awareness, no admin required)
- [ ] Create placeholder icons (`.ico` files for tray states)
- [ ] Add `.gitignore`, `LICENSE` (Apache-2.0)
- [ ] Verify project compiles on Windows

### 1.2 Configuration (`src/config.rs`)
- [ ] Define `Config` struct with serde derive
- [ ] TOML loading from `%APPDATA%\ImmichSync\config.toml`
- [ ] TOML saving with atomic write (write to temp, rename)
- [ ] Default config generation on first run
- [ ] Config directory creation if missing

### 1.3 State Database (`src/db.rs`)
- [ ] Initialize SQLite at `%APPDATA%\ImmichSync\state.db` with WAL mode
- [ ] Schema creation (watched_folders, uploaded_files, upload_queue, config tables)
- [ ] Create indexes on `uploaded_files.file_hash`, `uploaded_files.file_path`, `upload_queue.status`
- [ ] Schema migration system via `config.schema_version`
- [ ] CRUD operations for each table
- [ ] Dedup lookup by hash

### 1.4 Immich API Client (`src/api/`)
- [ ] `server.rs` — ping endpoint, server info, health check
- [ ] `auth.rs` — API key validation, connection test
- [ ] `assets.rs` — multipart file upload (`POST /api/assets`)
- [ ] `assets.rs` — bulk upload check (`POST /api/assets/bulk-upload-check`)
- [ ] `albums.rs` — create album, add assets to album
- [ ] Error types for API failures (auth, network, server errors)
- [ ] Configurable timeout and base URL

### 1.5 Watch Engine — Local Folders (`src/watch/`)
- [ ] `folder.rs` — `notify::RecommendedWatcher` with recursive watching
- [ ] `filter.rs` — file extension filtering (images + videos)
- [ ] `filter.rs` — exclusion list (Thumbs.db, desktop.ini, etc.)
- [ ] `filter.rs` — minimum file size check
- [ ] `mod.rs` — watch engine orchestrator, manages active watchers
- [ ] Debouncing via `notify-debouncer-full`
- [ ] Write completion detection (size stability + read lock check)

### 1.6 Upload Pipeline (`src/upload/`)
- [ ] `hasher.rs` — SHA-1 hashing with file read
- [ ] `queue.rs` — persistent queue backed by SQLite (transactional state transitions)
- [ ] `worker.rs` — async upload workers (tokio tasks)
- [ ] `metadata.rs` — EXIF date extraction, file timestamp fallback
- [ ] `mod.rs` — pipeline orchestrator: hash -> dedup check -> server bulk check -> queue -> upload -> record
- [ ] Configurable concurrency (default 2)
- [ ] Exponential backoff retry (1s, 2s, 4s... up to 32s, max 5 retries)
- [ ] Device asset ID generation (`{path_hash}-{filename}`)
- [ ] Device ID generation (`immichsync-{machine_name}`)

### 1.7 System Tray (`src/ui/tray.rs`)
- [ ] Tray icon initialization with `tray-icon` + `winit`
- [ ] Icon state switching (idle, syncing, error, offline)
- [ ] Right-click context menu (pause, settings, quit)
- [ ] Upload progress in menu (X/Y files)
- [ ] Server connection status in menu

### 1.8 Platform Basics (`src/platform/`)
- [ ] `single_instance.rs` — named mutex (`Global\ImmichSync`)
- [ ] `known_folders.rs` — `SHGetKnownFolderPath(FOLDERID_Pictures)` wrapper
- [ ] Auto-detect Pictures folder on first run

### 1.9 Application Lifecycle (`src/main.rs`, `src/app.rs`)
- [ ] Single-instance check on startup
- [ ] Load config, open database
- [ ] Start watch engine on configured folders
- [ ] Start upload pipeline
- [ ] Start tray icon
- [ ] Graceful shutdown on quit (flush queue, close watchers)

**MVP Exit Criteria:** Install the app, it watches your Pictures folder, uploads new photos/videos to Immich, shows progress in the tray. No settings UI yet — configure via TOML.

---

## Phase 2: Full Watch Support

### 2.1 Network Share Support (`src/watch/network.rs`)
- [ ] Detect UNC paths and mapped drives
- [ ] Health check: create temp file, wait for native event
- [ ] Auto-fallback to `PollWatcher` if no event within 5s
- [ ] Configurable poll interval per folder
- [ ] Log watcher mode per path

### 2.2 Removable Device Monitoring (`src/watch/device.rs`)
- [ ] Hidden message-only window for `WM_DEVICECHANGE`
- [ ] Handle `DBT_DEVICEARRIVAL`: resolve drive letter, enumerate volumes
- [ ] Handle `DBT_DEVICEREMOVECOMPLETE`: tear down watcher, flush queue
- [ ] DCIM folder auto-detection (cameras)
- [ ] Configurable: auto-watch / ask / ignore on insert

### 2.3 Platform — Drive Enumeration (`src/platform/drives.rs`)
- [ ] List available drives with type (fixed, removable, network)
- [ ] Detect drive letter from device notification
- [ ] Volume label reading for display

### 2.4 Watch Engine Enhancements
- [ ] Per-folder include/exclude glob patterns
- [ ] Per-folder album mode configuration
- [ ] Dynamic add/remove watchers at runtime (for settings changes)
- [ ] Watcher health monitoring and auto-restart

---

## Phase 3: Settings & Polish

### 3.1 Settings Window (`src/ui/settings.rs`)
- [ ] Connection tab: server URL, API key (masked input), test connection, device name
- [ ] Watch Folders tab: list with toggles, add/remove folders, per-folder config
- [ ] Devices tab: auto-watch toggle, DCIM toggle, insert action dropdown
- [ ] Upload tab: concurrency slider, bandwidth limit, upload order, retry count, file types
- [ ] Advanced tab: poll interval, log level, autostart, minimize-to-tray, db path, export/import

### 3.2 First-Run Setup (`src/ui/first_run.rs`)
- [ ] Welcome screen
- [ ] Server URL + API key entry with test
- [ ] Pictures folder confirmation
- [ ] Start-with-Windows option
- [ ] Done — start syncing

### 3.3 Album Auto-Creation
- [ ] Create album from parent folder name
- [ ] Create album from date (YYYY-MM)
- [ ] Add to fixed user-selected album
- [ ] Album caching to avoid redundant API calls

### 3.4 Notifications (`src/ui/notifications.rs`)
- [ ] Windows toast notifications
- [ ] Notify on batch upload complete
- [ ] Notify on error (server unreachable, auth failed)
- [ ] Configurable: enable/disable per notification type

### 3.5 Upload Log Viewer
- [ ] Simple scrollable list of recent uploads
- [ ] Show: filename, timestamp, status, error message
- [ ] Filter by status (completed, failed)

### 3.6 Bandwidth Throttling
- [ ] Token bucket rate limiter on upload stream
- [ ] Configurable KB/s limit (0 = unlimited)
- [ ] Dynamic adjustment via settings without restart

### 3.7 Auto-Start (`src/platform/autostart.rs`)
- [ ] Registry key at `HKCU\...\Run`
- [ ] Add/remove on settings toggle
- [ ] Detect current state on settings load

### 3.8 API Key Encryption
- [ ] AES-256-GCM encryption with DPAPI-derived key
- [ ] Encrypt on save, decrypt on load
- [ ] Fallback to plaintext with warning if DPAPI unavailable

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

### 4.1 Installer
- [ ] WiX installer configuration
- [ ] Start menu shortcut
- [ ] Optional desktop shortcut
- [ ] Uninstaller (clean registry, optionally remove data)

### 4.2 Build & Release Pipeline
- [ ] GitHub Actions CI (build, test, clippy, fmt)
- [ ] Release workflow: build -> sign -> create GitHub Release
- [ ] Portable ZIP alongside installer

### 4.3 Code Signing
- [ ] Obtain code signing certificate
- [ ] Sign executable and installer
- [ ] SmartScreen reputation building

### 4.4 Auto-Update
- [ ] Check GitHub Releases API for newer version
- [ ] Notify user via tray notification
- [ ] Download and apply update (or link to release page)

### 4.5 Documentation & Marketing
- [ ] Screenshots and demo GIF for README
- [ ] Landing page (simple GitHub Pages or standalone)
- [ ] Post to Immich community (GitHub Discussions, Reddit r/selfhosted)
- [ ] GitHub Sponsors / Buy Me a Coffee / Ko-fi setup

---

## Dependency Graph

```
Phase 1.1 (skeleton)
  -> 1.2 (config)
  -> 1.3 (database)
  -> 1.8 (platform basics)
     -> 1.4 (API client) [needs config for server URL]
     -> 1.5 (watch engine) [needs platform for Pictures folder]
        -> 1.6 (upload pipeline) [needs API client + database + watch events]
           -> 1.7 (tray) [needs upload pipeline for status]
              -> 1.9 (app lifecycle) [ties everything together]

Phase 2 depends on Phase 1 completion.
Phase 3 depends on Phase 1 completion (Phase 2 is independent).
Phase 4 depends on Phase 3 completion.
```

## Milestones

| Milestone | Description | Exit Criteria |
|-----------|-------------|---------------|
| **M1** | Compiles and runs | Tray icon appears, config loads |
| **M2** | Watches and uploads | New file in Pictures -> uploaded to Immich |
| **M3** | Reliable sync | Dedup works, retries work, survives crashes |
| **M4** | Full device support | Network shares and USB drives work |
| **M5** | Settings UI | All configuration via GUI |
| **M6** | Release-ready | Installer, signed, auto-update |
