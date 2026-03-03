#![windows_subsystem = "windows"]

mod app;
mod config;
mod db;

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
    // If not running from the installed location, show an install dialog
    // (unless portable_mode is set). The dialog handles copying the exe,
    // creating shortcuts, and setting autostart.
    match platform::is_running_installed() {
        Ok(false) => {
            // Check if user previously chose portable mode.
            let portable = config::Config::load()
                .map(|c| c.ui.portable_mode)
                .unwrap_or(false);

            if portable {
                info!("Portable mode enabled, skipping install");
            } else {
                info!("Not running from installed path, showing install dialog");

                let exe = std::env::current_exe().expect("current_exe");
                let status = std::process::Command::new(&exe)
                    .args(["--window", "install"])
                    .status();

                match status {
                    Ok(s) if s.code() == Some(0) => {
                        // User clicked Install — relaunch from installed path.
                        info!("Install complete, relaunching from installed path");
                        if let Err(e) = platform::relaunch_installed() {
                            tracing::warn!(error = %e, "Relaunch failed, continuing from current location");
                        }
                        // relaunch_installed calls process::exit on success.
                        // If we get here, it failed — fall through.
                    }
                    Ok(_) => {
                        // User chose Run Portable (exit code 1) or closed dialog.
                        info!("User chose portable mode, continuing from current location");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to launch install dialog, continuing");
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Could not check install status, continuing");
        }
        Ok(true) => {}
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

/// Run a single UI window in subprocess mode and exit.
///
/// Called when the binary is launched with `--window <type>`.  Each
/// subprocess gets its own winit EventLoop, sidestepping the winit 0.30
/// limitation that only allows one EventLoop per process lifetime.
fn run_window_subprocess(window_type: &str) -> anyhow::Result<()> {
    match window_type {
        "install" => {
            info!("Subprocess: running install dialog");
            ui::install::run_install_dialog();
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
