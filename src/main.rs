#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod db;
mod model;
mod settings;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rust DB Viewer")
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([800.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rust DB Viewer",
        options,
        Box::new(|cc| Ok(Box::new(app::ViewerApp::new(cc)))),
    )
}
