// Application state and lifecycle management.
//
// `App` wires together the upload pipeline, watch engine, system tray, and
// settings UI.  The main loop is a native Win32 message pump — simpler and
// more reliable for a tray-only app than running a full winit event loop.

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
    runtime: Arc<tokio::runtime::Runtime>,
    last_stats_update: Instant,
    paused: bool,
    /// Track previous syncing state to detect transitions for notifications.
    was_syncing: bool,
    /// Notification manager.
    notifications: crate::ui::notifications::Notifications,
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
            runtime,
            last_stats_update: Instant::now(),
            paused: false,
            was_syncing: false,
            notifications,
        }
    }

    /// Initialize the tray icon, upload pipeline, and watch engine.
    ///
    /// Must be called on the main thread before [`run`].
    pub fn init(&mut self) -> anyhow::Result<()> {
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

            // Periodically refresh tray with queue statistics.
            self.update_tray_stats();

            // Sleep to avoid busy-waiting (~20 wakes/sec is plenty).
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    // ── internals ────────────────────────────────────────────────────────

    /// Register watched folders and spawn the watch→pipeline bridge task.
    fn start_watch_engine(&mut self) -> anyhow::Result<()> {
        let mut engine = WatchEngine::new();
        let event_rx = engine
            .event_receiver()
            .ok_or_else(|| anyhow::anyhow!("event_receiver already taken"))?;

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
                        if let Ok(_id) = db.add_folder(
                            &pictures.display().to_string(),
                            Some("Pictures"),
                            true,
                        ) {
                            let filter = FileFilter::new();
                            if let Err(e) =
                                engine.add_folder(pictures, filter, false, self.runtime.handle().clone())
                            {
                                warn!("Failed to add Pictures watcher: {}", e);
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
                    if let Err(e) = engine.add_folder(path, filter, is_network, self.runtime.handle().clone())
                    {
                        warn!(path = %folder.path, error = %e, "Failed to add watcher");
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

            self.runtime.spawn(async move {
                bridge_watch_to_pipeline(event_rx, store, concurrency).await;
            });
        }

        Ok(())
    }

    /// Dispatch a tray menu action (everything except Quit).
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
                // Run on main thread — eframe's event loop pumps all Win32
                // messages, keeping the tray icon responsive while open.
                let config = self.config.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                crate::ui::settings::show_settings(config, Some(tx));
                // Settings window has closed. Apply config if user clicked Save.
                if let Ok(new_config) = rx.try_recv() {
                    self.apply_new_config(new_config);
                }
            }
            TrayAction::UploadNow => {
                info!("Upload Now requested");
            }
            TrayAction::About => {
                crate::ui::about::show_about();
            }
            TrayAction::ViewLog => {
                crate::ui::upload_log::show_upload_log(self.db.clone());
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
) {
    let queue = UploadQueue::new(store, concurrency);

    while let Some(event) = event_rx.recv().await {
        match event {
            WatchEvent::FileReady(path) => {
                info!(path = %path.display(), "Watch: file ready");
                match queue.process_file(path, None) {
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
