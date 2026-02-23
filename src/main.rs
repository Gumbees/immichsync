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
    if config.server.url.is_empty() {
        info!("Server not configured, launching first-run wizard");
        match ui::first_run::run_first_run_wizard() {
            Some(new_config) => {
                config = new_config;
                info!("First-run wizard completed");
            }
            None => {
                info!("First-run wizard cancelled, continuing with defaults");
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
