// About dialog — shows app info and a "Support ImmichSync" button.

use eframe::egui;

const STRIPE_LINK: &str = "https://buy.stripe.com/8x214n0IjaoL0zwcsj4AU00";

/// Open the About dialog on a separate thread (non-blocking).
pub fn show_about() {
    std::thread::spawn(|| {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([340.0, 220.0])
                .with_title("About ImmichSync")
                .with_resizable(false),
            ..Default::default()
        };

        let _ = eframe::run_native(
            "About ImmichSync",
            options,
            Box::new(|_cc| Ok(Box::new(AboutApp))),
        );
    });
}

struct AboutApp;

impl eframe::App for AboutApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(16.0);
                ui.heading("ImmichSync");
                ui.add_space(4.0);
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(8.0);
                ui.label("Watches folders and uploads photos/videos");
                ui.label("to your Immich server automatically.");
                ui.add_space(16.0);

                if ui.button("Support ImmichSync").clicked() {
                    let _ = open::that(STRIPE_LINK);
                }

                ui.add_space(8.0);

                if ui.button("Close").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}
