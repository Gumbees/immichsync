// Platform layer — Windows-specific integrations.
//
// All submodules are Windows-only.  No cross-platform abstractions are
// provided; the entire application targets Windows 10/11.

pub mod autostart;
pub mod drives;
pub mod encryption;
pub mod install;
pub mod known_folders;
pub mod single_instance;

// Re-export the most-used public types at the platform crate boundary so
// callers can write `platform::SingleInstance` instead of
// `platform::single_instance::SingleInstance`.

pub use autostart::{is_autostart_enabled, set_autostart, AutostartError};
pub use drives::{has_dcim_folder, list_drives, DriveInfo, DriveType};
pub use encryption::{decrypt_api_key, encrypt_api_key, is_encrypted, EncryptionError};
pub use install::{install_exe, installed_exe_path, is_running_installed, migrate_legacy_data, relaunch_installed};
pub use known_folders::{get_pictures_folder, KnownFolderError};
pub use single_instance::{SingleInstance, SingleInstanceError};
