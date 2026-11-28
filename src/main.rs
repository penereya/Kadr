#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod media;
mod photo;
mod theme;
mod updater;
mod video;

use eframe::egui;

fn create_app_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/app_icon.png");
    let img = image::load_from_memory(bytes)
        .expect("embedded app_icon.png must be valid PNG")
        .into_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kadr — Медиаредактор")
            .with_inner_size([1140.0, 720.0])
            .with_min_inner_size([860.0, 560.0])
            .with_drag_and_drop(true)
            .with_icon(create_app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Kadr",
        options,
        Box::new(|cc| Ok(Box::new(app::KadrApp::new(cc)))),
    )
}
