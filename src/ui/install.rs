// Install dialog — shown when the app is run from outside its install directory.
//
// Offers three checkboxes (autostart, desktop shortcut, start menu shortcut)
// and two buttons (Install, Run Portable).
//
// In update mode, the heading, button label, and description change to
// reflect that an existing installation is being updated.

use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::config::Config;

/// Result of the install dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallResult {
    /// User clicked Install/Update — exe was copied, shortcuts created.
    Installed,
    /// User clicked Run Portable or closed the window.
    Portable,
}

/// Run the install dialog. Blocks the calling thread.
///
/// When `is_update` is true, the dialog shows "Update ImmichSync" instead
/// of "Install ImmichSync" and displays the version transition.
///
/// Returns `InstallResult::Installed` if the user clicked Install/Update,
/// or `InstallResult::Portable` if Run Portable was chosen or the window
/// was closed.
///
/// When called from a subprocess (`--window install`), use
/// `run_install_dialog_subprocess` instead, which calls `process::exit`.
pub fn run_install_dialog(is_update: bool, old_version: Option<String>) -> InstallResult {
    let install_dir = Config::data_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unknown)".to_string());

    let title = if is_update {
        "Update ImmichSync"
    } else {
        "Install ImmichSync"
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 300.0])
            .with_title(title)
            .with_resizable(false)
            .with_always_on_top(),
        ..Default::default()
    };

    let result = Arc::new(Mutex::new(InstallResult::Portable));

    let app = InstallApp {
        install_dir,
        is_update,
        old_version,
        new_version: crate::platform::install::running_version().to_string(),
        autostart: true,
        desktop_shortcut: true,
        start_menu_shortcut: true,
        status: String::new(),
        result: result.clone(),
    };

    let _ = eframe::run_native(
        title,
        options,
        Box::new(|_cc| Ok(Box::new(app))),
    );

    // Return the result set by the dialog.
    let r = *result.lock().unwrap();
    r
}

/// Run the install dialog in subprocess mode (calls `process::exit`).
///
/// Used when invoked via `--window install` or `--window install-update`.
pub fn run_install_dialog_subprocess(is_update: bool, old_version: Option<String>) {
    let result = run_install_dialog(is_update, old_version);
    match result {
        InstallResult::Installed => std::process::exit(0),
        InstallResult::Portable => std::process::exit(1),
    }
}

struct InstallApp {
    install_dir: String,
    is_update: bool,
    old_version: Option<String>,
    new_version: String,
    autostart: bool,
    desktop_shortcut: bool,
    start_menu_shortcut: bool,
    status: String,
    result: Arc<Mutex<InstallResult>>,
}

impl eframe::App for InstallApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(16.0);

            ui.vertical_centered(|ui| {
                if self.is_update {
                    ui.heading("Update ImmichSync");
                } else {
                    ui.heading("Install ImmichSync");
                }
            });

            ui.add_space(12.0);

            if self.is_update {
                if let Some(ref old) = self.old_version.as_ref().filter(|v| *v != "unknown") {
                    ui.label(format!("Updating from v{old} to v{}", self.new_version));
                } else {
                    ui.label(format!("Updating to v{}", self.new_version));
                }
                ui.add_space(4.0);
            }

            ui.label("ImmichSync will be installed to:");
            ui.monospace(&self.install_dir);

            ui.add_space(16.0);

            ui.checkbox(&mut self.autostart, "Start with Windows");
            ui.checkbox(&mut self.desktop_shortcut, "Create desktop shortcut");
            ui.checkbox(&mut self.start_menu_shortcut, "Add to Start Menu");

            if !self.status.is_empty() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::RED, &self.status);
            }

            ui.add_space(16.0);

            let action_label = if self.is_update { "Update" } else { "Install" };

            ui.horizontal(|ui| {
                // Right-align buttons by adding flexible space first.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Run Portable").clicked() {
                        self.do_portable(ctx);
                    }

                    if ui.button(action_label).clicked() {
                        self.do_install(ctx);
                    }
                });
            });
        });
    }
}

impl InstallApp {
    fn do_install(&mut self, ctx: &egui::Context) {
        // Copy exe to install directory.
        if let Err(e) = crate::platform::install_exe() {
            self.status = format!("Install failed: {e}");
            return;
        }

        let installed_path = match crate::platform::installed_exe_path() {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Could not determine install path: {e}");
                return;
            }
        };

        // Create shortcuts.
        if self.desktop_shortcut {
            if let Err(e) = crate::platform::create_desktop_shortcut(
                &installed_path,
                "ImmichSync",
                "Sync photos and videos to Immich",
            ) {
                tracing::warn!(error = %e, "Failed to create desktop shortcut");
            }
        }

        if self.start_menu_shortcut {
            if let Err(e) = crate::platform::create_start_menu_shortcut(
                &installed_path,
                "ImmichSync",
                "Sync photos and videos to Immich",
            ) {
                tracing::warn!(error = %e, "Failed to create Start Menu shortcut");
            }
        }

        // Set autostart.
        if let Err(e) = crate::platform::set_autostart(self.autostart) {
            tracing::warn!(error = %e, "Failed to set autostart");
        }

        // Save config with portable_mode = false.
        if let Ok(mut config) = Config::load() {
            config.ui.start_with_windows = self.autostart;
            config.ui.portable_mode = false;
            if let Err(e) = config.save() {
                tracing::warn!(error = %e, "Failed to save config from install dialog");
            }
        }

        *self.result.lock().unwrap() = InstallResult::Installed;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn do_portable(&mut self, ctx: &egui::Context) {
        // Save config with portable_mode = true so we don't ask again.
        if let Ok(mut config) = Config::load() {
            config.ui.portable_mode = true;
            if let Err(e) = config.save() {
                tracing::warn!(error = %e, "Failed to save config from install dialog");
            }
        }

        *self.result.lock().unwrap() = InstallResult::Portable;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
