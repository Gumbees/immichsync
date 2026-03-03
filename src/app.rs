// Application state and lifecycle management.
//
// `App` wires together the upload pipeline, watch engine, system tray, and
// settings UI.  The main loop is a native Win32 message pump — simpler and
// more reliable for a tray-only app than running a full winit event loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, warn};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
};

use crate::api::ImmichClient;
use crate::config::Config;
use crate::db::DbStore;
use crate::ui::tray::{TrayAction, TrayApp, TrayState};
use crate::upload::queue::{QueueStore, UploadQueue};
use crate::upload::worker::AssetUploader;
use crate::upload::UploadPipeline;
use crate::watch::filter::FileFilter;
use crate::watch::folder::WatchEvent;
use crate::watch::WatchEngine;

/// Top-level application orchestrator.
pub struct App {
    config: Config,
    db: Arc<DbStore>,
    client: Option<ImmichClient>,
    pipeline: Option<UploadPipeline>,
    watch_engine: Option<WatchEngine>,
    tray: Option<TrayApp>,
    tray_rx: Option<std::sync::mpsc::Receiver<TrayAction>>,
    /// Receiver for config updates from the settings window (spawned on another thread).
    settings_rx: Option<std::sync::mpsc::Receiver<Config>>,
    runtime: Arc<tokio::runtime::Runtime>,
    last_stats_update: Instant,
    paused: bool,
    /// Track previous syncing state to detect transitions for notifications.
    was_syncing: bool,
    /// Notification manager.
    notifications: crate::ui::notifications::Notifications,
    /// Tracks whether each UI window type is currently open.
    window_open: WindowOpenTracker,
}

/// Tracks whether each type of UI window is currently open, preventing
/// multiple instances of the same window from being spawned.
#[derive(Clone)]
pub struct WindowOpenTracker {
    pub settings: Arc<AtomicBool>,
    pub about: Arc<AtomicBool>,
    pub upload_log: Arc<AtomicBool>,
}

impl WindowOpenTracker {
    fn new() -> Self {
        Self {
            settings: Arc::new(AtomicBool::new(false)),
            about: Arc::new(AtomicBool::new(false)),
            upload_log: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl App {
    /// Create a new app instance. Call [`init`] then [`run`].
    pub fn new(
        config: Config,
        db: Arc<DbStore>,
        client: Option<ImmichClient>,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Self {
        let notifications = crate::ui::notifications::Notifications::new(&config);
        Self {
            config,
            db,
            client,
            pipeline: None,
            watch_engine: None,
            tray: None,
            tray_rx: None,
            settings_rx: None,
            runtime,
            last_stats_update: Instant::now(),
            paused: false,
            was_syncing: false,
            notifications,
            window_open: WindowOpenTracker::new(),
        }
    }

    /// Initialize the tray icon, upload pipeline, and watch engine.
    ///
    /// Must be called on the main thread before [`run`].
    pub fn init(&mut self) -> anyhow::Result<()> {
        // Recover any entries stuck in "uploading" state from a previous crash.
        {
            let db = self.db.inner().lock().unwrap();
            if let Err(e) = db.reset_stale_uploading() {
                warn!(error = %e, "Failed to reset stale uploading entries");
            }
        }

        // Create system tray (must be on main thread).
        let (tray, tray_rx) = TrayApp::new()
            .map_err(|e| anyhow::anyhow!("Failed to create tray: {}", e))?;
        self.tray = Some(tray);
        self.tray_rx = Some(tray_rx);

        // Create and start the upload pipeline if server is configured.
        if let Some(ref client) = self.client {
            let uploader: Arc<dyn AssetUploader> = Arc::new(client.clone());
            let store: Arc<dyn QueueStore> = self.db.clone();
            let mut pipeline = UploadPipeline::new(
                store,
                &self.config.server,
                &self.config.upload,
                uploader,
            );

            // Pipeline spawns tokio tasks — enter the runtime context.
            let _guard = self.runtime.enter();
            pipeline
                .start()
                .map_err(|e| anyhow::anyhow!("Failed to start pipeline: {}", e))?;

            self.pipeline = Some(pipeline);

            // Update tray with server info.
            if let Some(ref mut tray) = self.tray {
                let url_display = self
                    .config
                    .server
                    .url
                    .trim_start_matches("https://")
                    .trim_start_matches("http://");
                tray.set_server_status(url_display, true);
            }
        } else {
            info!("No server configured; pipeline not started");
            if let Some(ref mut tray) = self.tray {
                tray.update_state(TrayState::Error(
                    "Server not configured".to_string(),
                ));
            }
        }

        // Set up the watch engine and bridge task.
        self.start_watch_engine()?;

        Ok(())
    }

    /// Run the main message loop. Blocks until the user selects Quit.
    ///
    /// Uses a native Win32 message pump so tray-icon's hidden window
    /// receives all its messages correctly (right-click menu, etc.).
    pub fn run(&mut self) {
        info!("Entering main message loop");

        loop {
            // Pump all pending Windows messages. This is what makes the
            // tray icon's context menu work — tray-icon creates a hidden
            // window that needs message dispatch to show the popup menu.
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).into() {
                    if msg.message == WM_QUIT {
                        info!("WM_QUIT received");
                        self.shutdown();
                        return;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            // Process tray menu actions. Collect first to avoid
            // borrow conflict (tray_rx is &self, handle needs &mut self).
            let actions: Vec<TrayAction> = self
                .tray_rx
                .as_ref()
                .map(|rx| std::iter::from_fn(|| rx.try_recv().ok()).collect())
                .unwrap_or_default();

            let mut quit = false;
            for action in actions {
                if action == TrayAction::Quit {
                    quit = true;
                } else {
                    self.handle_tray_action(action);
                }
            }
            if quit {
                info!("Quit requested from tray");
                self.shutdown();
                return;
            }

            // Check for config updates from the settings window.
            if let Some(ref rx) = self.settings_rx {
                if let Ok(new_config) = rx.try_recv() {
                    self.apply_new_config(new_config);
                }
            }

            // Periodically refresh tray with queue statistics.
            self.update_tray_stats();

            // Sleep to avoid busy-waiting (~20 wakes/sec is plenty).
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    // ── internals ────────────────────────────────────────────────────────

    /// Register watched folders and spawn the watch→pipeline bridge task.
    fn start_watch_engine(&mut self) -> anyhow::Result<()> {
        use std::collections::HashMap;

        let mut engine = WatchEngine::new();
        let event_rx = engine
            .event_receiver()
            .ok_or_else(|| anyhow::anyhow!("event_receiver already taken"))?;

        // Map from canonical watched path → folder DB id, so the bridge task
        // can look up which folder a file event belongs to.
        let mut path_to_folder_id: HashMap<std::path::PathBuf, i64> = HashMap::new();

        // Load watched folders from the database.
        {
            let db = self.db.inner().lock().unwrap();
            let folders = db.get_folders()?;

            if folders.is_empty() {
                drop(db);
                // First run: add the user's Pictures folder as a default.
                if let Ok(pictures) =
                    crate::platform::known_folders::get_pictures_folder()
                {
                    if pictures.exists() {
                        info!(path = %pictures.display(), "Adding default Pictures folder");
                        let db = self.db.inner().lock().unwrap();
                        if let Ok(id) = db.add_folder(
                            &pictures.display().to_string(),
                            Some("Pictures"),
                            true,
                        ) {
                            let filter = FileFilter::new();
                            let canon = std::fs::canonicalize(&pictures)
                                .unwrap_or_else(|_| pictures.clone());
                            if let Err(e) =
                                engine.add_folder(pictures, filter, false, self.runtime.handle().clone())
                            {
                                warn!("Failed to add Pictures watcher: {}", e);
                            } else {
                                path_to_folder_id.insert(canon, id);
                            }
                        }
                    }
                }
            } else {
                for folder in &folders {
                    if !folder.enabled {
                        continue;
                    }
                    let path = std::path::PathBuf::from(&folder.path);
                    if !path.exists() {
                        warn!(path = %folder.path, "Watch folder missing, skipping");
                        continue;
                    }
                    let is_network =
                        folder.watch_mode == crate::db::WatchMode::Poll;
                    let filter = FileFilter::new();
                    let canon = std::fs::canonicalize(&path)
                        .unwrap_or_else(|_| path.clone());
                    if let Err(e) = engine.add_folder(path, filter, is_network, self.runtime.handle().clone())
                    {
                        warn!(path = %folder.path, error = %e, "Failed to add watcher");
                    } else {
                        path_to_folder_id.insert(canon, folder.id);
                    }
                }
            }
        }

        info!(count = engine.watcher_count(), "Watch engine started");
        self.watch_engine = Some(engine);

        // Spawn the bridge task that feeds watch events into the upload queue.
        if self.pipeline.is_some() {
            let store: Arc<dyn QueueStore> = self.db.clone();
            let concurrency = self.config.upload.concurrency as usize;
            let folder_map = Arc::new(path_to_folder_id);

            info!("Spawning watch→pipeline bridge task (concurrency={})", concurrency);
            self.runtime.spawn(async move {
                bridge_watch_to_pipeline(event_rx, store, concurrency, folder_map).await;
            });
        } else {
            warn!("No upload pipeline — watch events will not be processed. Is the server configured?");
        }

        Ok(())
    }

    /// Dispatch a tray menu action (everything except Quit).
    ///
    /// UI windows (Settings, About, Upload Log) are spawned as child
    /// processes so each gets its own winit EventLoop — winit 0.30 only
    /// allows one EventLoop per process lifetime.
    fn handle_tray_action(&mut self, action: TrayAction) {
        match action {
            TrayAction::Pause => {
                if let Some(ref pipeline) = self.pipeline {
                    pipeline.pause();
                    self.paused = true;
                    if let Some(ref mut tray) = self.tray {
                        tray.update_state(TrayState::Paused);
                    }
                }
            }
            TrayAction::Resume => {
                if let Some(ref pipeline) = self.pipeline {
                    pipeline.resume();
                    self.paused = false;
                    if let Some(ref mut tray) = self.tray {
                        tray.update_state(TrayState::Idle);
                    }
                }
            }
            TrayAction::OpenSettings => {
                if self.window_open.settings.swap(true, Ordering::SeqCst) {
                    info!("Settings window already open, ignoring");
                    return;
                }
                let (tx, rx) = std::sync::mpsc::channel();
                self.settings_rx = Some(rx);
                let flag = self.window_open.settings.clone();
                spawn_window_subprocess("settings", flag, Some(tx));
            }
            TrayAction::UploadNow => {
                info!("Upload Now requested");
            }
            TrayAction::About => {
                if self.window_open.about.swap(true, Ordering::SeqCst) {
                    info!("About window already open, ignoring");
                    return;
                }
                let flag = self.window_open.about.clone();
                spawn_window_subprocess("about", flag, None);
            }
            TrayAction::ViewLog => {
                if self.window_open.upload_log.swap(true, Ordering::SeqCst) {
                    info!("Upload Log window already open, ignoring");
                    return;
                }
                let flag = self.window_open.upload_log.clone();
                spawn_window_subprocess("log", flag, None);
            }
            TrayAction::Quit => {
                // Handled in run() directly.
            }
        }
    }

    /// Apply a new config received from the settings window.
    fn apply_new_config(&mut self, new_config: Config) {
        info!("Applying config update from settings");

        let server_changed = new_config.server.url != self.config.server.url
            || new_config.server.api_key != self.config.server.api_key;

        // Apply the new config.
        self.config = new_config;
        self.notifications = crate::ui::notifications::Notifications::new(&self.config);

        if server_changed {
            info!("Server config changed, recreating client");

            // Recreate the Immich client.
            if !self.config.server.url.is_empty() && !self.config.server.api_key.is_empty() {
                match ImmichClient::new(&self.config.server.url, &self.config.server.api_key) {
                    Ok(c) => {
                        self.client = Some(c);
                        if let Some(ref mut tray) = self.tray {
                            let url_display = self
                                .config
                                .server
                                .url
                                .trim_start_matches("https://")
                                .trim_start_matches("http://");
                            tray.set_server_status(url_display, true);
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to create new Immich client");
                        self.client = None;
                    }
                }
            }
        }

        // Reload watched folders by stopping and restarting the watch engine.
        if let Some(ref mut engine) = self.watch_engine {
            engine.stop_all();
        }
        self.watch_engine = None;

        if let Err(e) = self.start_watch_engine() {
            warn!(error = %e, "Failed to restart watch engine after config update");
        }
    }

    /// Periodically update the tray icon with queue stats.
    fn update_tray_stats(&mut self) {
        if self.last_stats_update.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_stats_update = Instant::now();

        if self.paused {
            return;
        }

        if let Some(ref pipeline) = self.pipeline {
            if let Ok(stats) = pipeline.stats() {
                if let Some(ref mut tray) = self.tray {
                    let active = stats.uploading + stats.pending;
                    let is_syncing = active > 0;

                    if is_syncing {
                        tray.update_state(TrayState::Syncing {
                            current: stats.uploading as u32,
                            total: active as u32,
                        });
                    } else {
                        tray.update_state(TrayState::Idle);
                    }

                    // Detect Syncing → Idle transition for notification.
                    if self.was_syncing && !is_syncing && stats.completed > 0 {
                        self.notifications.notify_upload_complete(stats.completed as u32);
                    }
                    self.was_syncing = is_syncing;
                }
            }
        }
    }

    /// Stop all background work.
    fn shutdown(&mut self) {
        info!("Shutting down");

        if let Some(ref mut engine) = self.watch_engine {
            engine.stop_all();
        }

        if let Some(mut pipeline) = self.pipeline.take() {
            let rt = self.runtime.clone();
            rt.block_on(async {
                pipeline.stop().await;
            });
        }

        info!("Shutdown complete");
    }
}

// ─── Watch → Pipeline bridge ─────────────────────────────────────────────────

/// Async task that reads watch events and submits new files to the upload queue.
async fn bridge_watch_to_pipeline(
    mut event_rx: tokio::sync::mpsc::Receiver<WatchEvent>,
    store: Arc<dyn QueueStore>,
    concurrency: usize,
    folder_map: Arc<std::collections::HashMap<std::path::PathBuf, i64>>,
) {
    let queue = UploadQueue::new(store, concurrency);

    while let Some(event) = event_rx.recv().await {
        match event {
            WatchEvent::FileReady(path) => {
                info!(path = %path.display(), "Watch: file ready");
                let folder_id = resolve_folder_id(&path, &folder_map);
                match queue.process_file(path, folder_id) {
                    Ok(Some(id)) => info!(id, "File enqueued"),
                    Ok(None) => info!("File already uploaded, skipping"),
                    Err(e) => warn!(error = %e, "Failed to process file"),
                }
            }
            WatchEvent::FileRemoved(path) => {
                info!(path = %path.display(), "Watch: file removed (ignoring)");
            }
            WatchEvent::Error(msg) => {
                warn!(error = %msg, "Watch engine error");
            }
        }
    }
    info!("Watch→pipeline bridge stopped");
}

/// Spawn a UI window as a child process.
///
/// Each child process runs `immichsync.exe --window <type>`, giving it its
/// own winit EventLoop.  A background thread waits for the subprocess to
/// exit, then clears the `open_flag` and optionally reloads config from
/// disk (for settings).
fn spawn_window_subprocess(
    window_type: &'static str,
    open_flag: Arc<AtomicBool>,
    config_tx: Option<std::sync::mpsc::Sender<crate::config::Config>>,
) {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "Failed to get current exe path");
            open_flag.store(false, Ordering::SeqCst);
            return;
        }
    };

    std::thread::spawn(move || {
        info!(window_type, "Spawning window subprocess");
        match std::process::Command::new(&exe)
            .args(["--window", window_type])
            .status()
        {
            Ok(status) => {
                info!(window_type, ?status, "Window subprocess exited");
                // For settings, reload config from disk after the subprocess exits.
                if let Some(tx) = config_tx {
                    if let Ok(config) = crate::config::Config::load() {
                        let _ = tx.send(config);
                    }
                }
            }
            Err(e) => {
                warn!(window_type, error = %e, "Failed to spawn window subprocess");
            }
        }
        open_flag.store(false, Ordering::SeqCst);
    });
}

/// Find the watched folder that contains `file_path` by walking up the
/// directory tree and checking against the canonical folder map.
fn resolve_folder_id(
    file_path: &std::path::Path,
    folder_map: &std::collections::HashMap<std::path::PathBuf, i64>,
) -> Option<i64> {
    let canonical = std::fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
    let mut dir = canonical.parent();
    while let Some(d) = dir {
        if let Some(&id) = folder_map.get(d) {
            return Some(id);
        }
        dir = d.parent();
    }
    None
}
