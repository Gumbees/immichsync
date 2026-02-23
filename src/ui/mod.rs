// UI layer — system tray, settings window, notifications, first-run wizard,
// about dialog, upload log viewer.

pub mod about;
pub mod first_run;
pub mod notifications;
pub mod settings;
pub mod tray;
pub mod upload_log;

// Re-export the primary public surface so callers can write `ui::TrayApp`
// instead of `ui::tray::TrayApp`.

pub use first_run::FirstRun;
pub use notifications::Notifications;
pub use settings::Settings;
pub use tray::{TrayAction, TrayApp, TrayError, TrayState};
