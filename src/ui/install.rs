// Install dialog — shown when the app is run from outside its install directory.
//
// Offers three checkboxes (autostart, desktop shortcut, start menu shortcut)
// and two buttons (Install, Run Portable).
//
// Exit codes:
//   0 = Install chosen — caller should relaunch from installed path
//   1 = Run Portable — caller should continue from current location

use eframe::egui;

use crate::config::Config;

/// Run the install dialog. Blocks the calling thread.
///
/// Returns exit code `0` if the user clicked Install (exe was copied,
/// shortcuts created), or `1` if Run Portable was chosen.
pub fn run_install_dialog() {
    let install_dir = Config::data_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unknown)".to_string());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 280.0])
            .with_title("Install ImmichSync")
            .with_resizable(false),
        ..Default::default()
    };

    let app = InstallApp {
        install_dir,
        autostart: true,
        desktop_shortcut: true,
        start_menu_shortcut: true,
        status: String::new(),
    };

    let _ = eframe::run_native(
        "Install ImmichSync",
        options,
        Box::new(|_cc| Ok(Box::new(app))),
    );

    // If eframe exits without calling process::exit (user closed the window),
    // treat it like Run Portable.
    std::process::exit(1);
}

struct InstallApp {
    install_dir: String,
    autostart: bool,
    desktop_shortcut: bool,
    start_menu_shortcut: bool,
    status: String,
}

impl eframe::App for InstallApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(16.0);

            ui.vertical_centered(|ui| {
                ui.heading("Install ImmichSync");
            });

            ui.add_space(12.0);

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

            ui.horizontal(|ui| {
                // Right-align buttons by adding flexible space first.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Run Portable").clicked() {
                        self.do_portable(ctx);
                    }

                    if ui.button("Install").clicked() {
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

        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        std::process::exit(0);
    }

    fn do_portable(&mut self, ctx: &egui::Context) {
        // Save config with portable_mode = true so we don't ask again.
        if let Ok(mut config) = Config::load() {
            config.ui.portable_mode = true;
            if let Err(e) = config.save() {
                tracing::warn!(error = %e, "Failed to save config from install dialog");
            }
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        std::process::exit(1);
    }
}
