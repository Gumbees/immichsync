#![windows_subsystem = "windows"]

mod app;
mod config;
mod db;
mod updater;

mod api;
mod platform;
mod ui;
mod upload;
mod watch;

use std::sync::Arc;

use tracing::info;

fn main() -> anyhow::Result<()> {
    // ── Legacy data migration ────────────────────────────────────────────
    // Move config/db/logs from old %APPDATA%\ImmichSync\ to the new
    // %APPDATA%\bees-roadhouse\immichsync\ path before anything else
    // tries to read them.
    if let Err(e) = platform::migrate_legacy_data() {
        // Non-fatal: log to stderr since tracing isn't up yet.
        eprintln!("Warning: legacy data migration failed: {e}");
    }

    // Clean up leftover .exe.old from a previous self-update.
    updater::cleanup_old_exe();

    // ── Logging ──────────────────────────────────────────────────────────
    let data_dir = config::Config::data_dir()?;
    let log_dir = data_dir.join("logs");
    std::fs::create_dir_all(&log_dir)?;

    let file_appender =
        tracing_appender::rolling::daily(&log_dir, "immichsync.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    // ── Subprocess window mode ──────────────────────────────────────────
    // When launched with `--window <type>`, run only that UI window and
    // exit.  Each subprocess gets its own winit EventLoop, avoiding the
    // "EventLoop can't be recreated" limitation of winit 0.30.
    //
    // IMPORTANT: This must come before the install check, otherwise
    // `--window install` would try to show the install dialog recursively.
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 && args[1] == "--window" {
        return run_window_subprocess(&args[2]);
    }

    // ── Install dialog + relaunch ────────────────────────────────────────
    // If not running from the installed location, show an install/update
    // dialog DIRECTLY in this process (not as a subprocess). Running it
    // inline guarantees the window gets foreground focus — subprocess
    // windows on Windows often open behind other windows without focus.
    //
    // This is safe because the main process hasn't created a winit
    // EventLoop yet, and after this point it only uses a Win32 message
    // pump for the tray app.
    match platform::is_running_installed() {
        Ok(false) => {
            // Check if user previously chose portable mode.
            let portable = config::Config::load()
                .map(|c| c.ui.portable_mode)
                .unwrap_or(false);

            if portable {
                info!("Portable mode enabled, skipping install");
            } else {
                let installed_exe = platform::installed_exe_path().ok();
                let installed_exists = installed_exe.as_ref().map_or(false, |p| p.exists());

                if installed_exists {
                    // Determine if the installed copy needs updating.
                    let installed_ver = platform::install::installed_version();
                    let update_info = match &installed_ver {
                        None => {
                            Some((String::from("unknown"), platform::install::running_version().to_string()))
                        }
                        Some(_) => platform::install::is_update_available(),
                    };

                    if let Some((old_ver, new_ver)) = update_info {
                        // Running version is newer — show update dialog inline.
                        info!(
                            from = %old_ver, to = %new_ver,
                            "Newer version running, showing install-update dialog"
                        );

                        let result = ui::install::run_install_dialog(true, Some(old_ver));

                        match result {
                            ui::install::InstallResult::Installed => {
                                info!("Update complete, relaunching from installed path");
                                if let Err(e) = platform::relaunch_installed() {
                                    tracing::warn!(error = %e, "Relaunch failed, continuing from current location");
                                }
                            }
                            ui::install::InstallResult::Portable => {
                                info!("User declined update, continuing from current location");
                            }
                        }
                    } else {
                        // Same version already installed — silently relaunch.
                        info!("Same version already installed, relaunching from installed path");
                        if let Err(e) = platform::relaunch_installed() {
                            tracing::warn!(error = %e, "Relaunch failed, continuing from current location");
                        }
                    }
                } else {
                    // No installed copy — show fresh install dialog inline.
                    info!("Not running from installed path, showing install dialog");

                    let result = ui::install::run_install_dialog(false, None);

                    match result {
                        ui::install::InstallResult::Installed => {
                            info!("Install complete, relaunching from installed path");
                            if let Err(e) = platform::relaunch_installed() {
                                tracing::warn!(error = %e, "Relaunch failed, continuing from current location");
                            }
                        }
                        ui::install::InstallResult::Portable => {
                            info!("User chose portable mode, continuing from current location");
                        }
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Could not check install status, continuing");
        }
        Ok(true) => {
            // Running from installed location — check for updates before starting the tray app.
            info!("Running from installed path, checking for updates");
            check_for_update_on_startup();
        }
    }

    info!("ImmichSync starting");

    // ── Single-instance check ────────────────────────────────────────────
    let _instance = match platform::SingleInstance::acquire() {
        Ok(Some(guard)) => guard,
        Ok(None) => return Ok(()),
        Err(e) => {
            tracing::error!("Single-instance check failed: {}", e);
            return Err(e.into());
        }
    };

    // ── Config ───────────────────────────────────────────────────────────
    let mut config = config::Config::load()?;
    info!("Config loaded");

    // ── First-run wizard ────────────────────────────────────────────────
    // Runs as a child process so it gets its own winit EventLoop, leaving
    // the main process free to spawn further UI windows later.
    if config.server.url.is_empty() {
        info!("Server not configured, launching first-run wizard");
        let exe = platform::installed_exe_path()
            .map(|p| if p.exists() { p } else { std::env::current_exe().unwrap_or(p) })
            .unwrap_or_else(|_| std::env::current_exe().expect("current_exe"));
        let status = std::process::Command::new(&exe)
            .args(["--window", "wizard"])
            .status();
        match status {
            Ok(s) if s.success() => {
                // Reload config — the wizard saves to disk on completion.
                config = config::Config::load()?;
                if config.server.url.is_empty() {
                    info!("First-run wizard cancelled, continuing with defaults");
                } else {
                    info!("First-run wizard completed");
                }
            }
            Ok(s) => {
                tracing::warn!(code = ?s.code(), "First-run wizard exited abnormally");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to launch first-run wizard");
            }
        }
    }

    // ── Database ─────────────────────────────────────────────────────────
    let database = db::Database::open()?;
    let db_store = Arc::new(db::DbStore::new(database));

    // ── Immich client ────────────────────────────────────────────────────
    let client = if !config.server.url.is_empty()
        && !config.server.api_key.is_empty()
    {
        match api::ImmichClient::with_bandwidth_limit(
            &config.server.url,
            &config.server.api_key,
            config.upload.bandwidth_limit_kbps,
        ) {
            Ok(c) => {
                info!(url = %config.server.url, "Immich client created");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to create Immich client: {}; continuing without uploads",
                    e
                );
                None
            }
        }
    } else {
        info!("Server not configured; running without uploads");
        None
    };

    // ── Tokio runtime ────────────────────────────────────────────────────
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()?,
    );

    // ── App lifecycle ────────────────────────────────────────────────────
    let mut app = app::App::new(config, db_store, client, runtime);
    app.init()?;
    app.run(); // blocks until Quit

    Ok(())
}

/// Blocking update check at startup (before the tray app starts).
///
/// Runs a quick GitHub API check. If an update is available, serializes the
/// info to a temp file, spawns the update dialog as a subprocess, and waits.
/// If the update was applied (exit code 0), relaunches self.  Otherwise
/// continues to the normal tray app.
fn check_for_update_on_startup() {
    // Load config to check if updates are enabled.
    let updates_enabled = config::Config::load()
        .map(|c| c.advanced.check_for_updates)
        .unwrap_or(true);

    if !updates_enabled {
        info!("Update checks disabled in config, skipping startup check");
        return;
    }

    // Build a temporary tokio runtime for the async check.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to create runtime for update check");
            return;
        }
    };

    let result = rt.block_on(updater::check_for_update());

    match result {
        updater::UpdateCheckResult::Available(info) => {
            info!(
                current = %info.current_version,
                new = %info.new_version,
                "Update available at startup, showing update dialog"
            );

            // Serialize UpdateInfo to a temp JSON file.
            let temp_dir = std::env::temp_dir();
            let info_path = temp_dir.join("immichsync_update_info.json");
            let info_json = match serde_json::to_string(&info) {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to serialize update info");
                    return;
                }
            };
            if let Err(e) = std::fs::write(&info_path, &info_json) {
                tracing::warn!(error = %e, "Failed to write update info file");
                return;
            }

            // Spawn the update dialog subprocess.
            let exe = std::env::current_exe().expect("current_exe");
            let info_path_str = info_path.display().to_string();

            info!("Spawning update dialog subprocess");
            match std::process::Command::new(&exe)
                .args(["--window", "update", "--update-info", &info_path_str])
                .status()
            {
                Ok(status) if status.code() == Some(0) => {
                    // Update was applied — relaunch self.
                    info!("Startup update applied, relaunching");
                    updater::relaunch_self();
                }
                Ok(status) => {
                    info!(code = ?status.code(), "User skipped startup update, continuing to tray app");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to launch update dialog");
                }
            }
        }
        updater::UpdateCheckResult::UpToDate => {
            info!("Already on latest version at startup");
        }
        updater::UpdateCheckResult::Failed(msg) => {
            tracing::warn!(error = %msg, "Startup update check failed, continuing");
        }
    }
}

/// Run a single UI window in subprocess mode and exit.
///
/// Called when the binary is launched with `--window <type>`.  Each
/// subprocess gets its own winit EventLoop, sidestepping the winit 0.30
/// limitation that only allows one EventLoop per process lifetime.
fn run_window_subprocess(window_type: &str) -> anyhow::Result<()> {
    match window_type {
        "install" => {
            info!("Subprocess: running install dialog");
            ui::install::run_install_dialog_subprocess(false, None);
        }
        "install-update" => {
            info!("Subprocess: running update dialog");
            // Parse --old-version from remaining args.
            let args: Vec<String> = std::env::args().collect();
            let old_version = args
                .windows(2)
                .find(|w| w[0] == "--old-version")
                .map(|w| w[1].clone());
            ui::install::run_install_dialog_subprocess(true, old_version);
        }
        "wizard" => {
            info!("Subprocess: running first-run wizard");
            ui::first_run::run_first_run_wizard();
        }
        "settings" => {
            info!("Subprocess: running settings window");
            let config = config::Config::load()?;
            ui::settings::show_settings(config, None);
        }
        "about" => {
            info!("Subprocess: running about dialog");
            ui::about::show_about();
        }
        "update" => {
            info!("Subprocess: running update dialog");
            let args: Vec<String> = std::env::args().collect();
            let info_path = args
                .windows(2)
                .find(|w| w[0] == "--update-info")
                .map(|w| w[1].clone())
                .unwrap_or_default();
            if info_path.is_empty() {
                tracing::error!("--update-info path required for update window");
                std::process::exit(1);
            }
            ui::update::run_update_dialog(&info_path);
        }
        "log" => {
            info!("Subprocess: running upload log");
            let database = db::Database::open()?;
            let db_store = Arc::new(db::DbStore::new(database));
            ui::upload_log::show_upload_log(db_store);
        }
        other => {
            tracing::error!("Unknown window type: {other}");
        }
    }
    Ok(())
}
