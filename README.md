# ImmichSync

A lightweight, native Windows system tray application that watches folders for new photos and videos and automatically uploads them to your [Immich](https://immich.app) server.

Install it, point it at your server, forget it exists.

## Why ImmichSync?

Immich has 84K+ GitHub stars and a thriving community, but no official desktop sync client. The existing options are CLI scripts, janky Python wrappers, or closed-source tools. ImmichSync fills the gap: a native Windows app that sits in your system tray and Just Works.

- **Zero friction** — watches your Pictures folder by default
- **Handles everything** — local folders, network shares, USB drives, SD cards
- **Smart uploads** — deduplication, retry with backoff, bandwidth throttling
- **Tiny footprint** — pure Rust, ~5MB binary, no runtime dependencies
- **Set and forget** — starts with Windows, syncs in the background

## Features

- **Folder watching** — recursive filesystem monitoring via `ReadDirectoryChangesW`
- **Network share support** — automatic fallback to polling when native watching fails
- **Removable device detection** — auto-detect USB drives and SD cards with DCIM folders
- **Upload queue** — persistent, crash-resistant, with configurable concurrency
- **SHA-1 deduplication** — matches Immich's internal checksum, saves bandwidth
- **Album auto-creation** — organize by folder name, date, or custom album
- **EXIF-aware** — extracts photo dates for accurate timestamps
- **Encrypted API key storage** — secured via Windows DPAPI
- **System tray** — status icons, pause/resume, upload progress
- **Settings UI** — clean configuration window built with egui
- **Auto-start** — optional Windows startup via registry

## Installation

### Download

Grab the latest release from [GitHub Releases](https://github.com/gumbees/immichsync/releases).

Run the installer or extract the portable ZIP. That's it.

### Building from Source

**Prerequisites:**
- [Rust](https://rustup.rs/) (stable, 1.75+)
- Windows 10 or 11

```bash
git clone https://github.com/gumbees/immichsync.git
cd immichsync
cargo build --release
```

The binary lands in `target/release/immichsync.exe`.

## Quick Start

1. Launch ImmichSync — it appears in your system tray
2. Right-click the tray icon → **Settings**
3. Enter your Immich server URL and API key
4. Click **Test Connection** to verify
5. Your Pictures folder is watched by default — add more folders if needed
6. Close settings — ImmichSync is now syncing

### Getting Your API Key

1. Open your Immich web interface
2. Go to **User Settings** → **API Keys**
3. Click **New API Key**, give it a name, copy the key

## Configuration

Settings are stored in `%APPDATA%\bees-roadhouse\immichsync\config.toml`. You can edit this directly or use the Settings UI.

```toml
[server]
url = "https://immich.example.com"
api_key = "encrypted:base64..."

[upload]
concurrency = 2
bandwidth_limit_kbps = 0  # 0 = unlimited
order = "newest_first"
max_retries = 5

[devices]
auto_watch = true
look_for_dcim = true
on_insert = "auto"  # "auto" | "ask" | "ignore"

[ui]
start_with_windows = true
minimize_to_tray = true
show_notifications = true
```

## Supported File Types

**Images:** jpg, jpeg, png, gif, webp, heic, heif, avif, tiff, bmp, raw, cr2, cr3, nef, arw, dng, orf, rw2, pef, srw, raf

**Videos:** mp4, mov, avi, mkv, webm, m4v, 3gp, mts, m2ts

Custom include/exclude patterns are configurable per watch folder.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full technical design.

**Key components:**
- **Watch Engine** — `notify` crate with `ReadDirectoryChangesW`, poll fallback for network shares
- **Device Monitor** — Win32 `WM_DEVICECHANGE` for USB/SD card detection
- **Upload Pipeline** — async Tokio workers with persistent queue
- **State Database** — SQLite via `rusqlite` (bundled)
- **UI** — `tray-icon` + `egui` for system tray and settings window

## Contributing

Contributions are welcome. See [PLAN.md](PLAN.md) for the implementation roadmap and open tasks.

1. Fork the repo
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Commit your changes
4. Push and open a PR

Please keep PRs focused — one feature or fix per PR.

## Support the Project

ImmichSync is free and open source. If it saves you time, consider supporting development:

- [Donate via Stripe](https://buy.stripe.com/8x214n0IjaoL0zwcsj4AU00)

## License

GPL-3.0-only — see [LICENSE](LICENSE) for details.

## Acknowledgments

- [Immich](https://immich.app) — the excellent self-hosted photo platform this syncs to
- The Rust ecosystem — `notify`, `tokio`, `reqwest`, `egui`, `rusqlite`, and all the crates that make this possible
