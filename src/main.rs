#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod media;
mod photo;
mod theme;
mod video;

use eframe::egui;

fn create_app_icon() -> egui::IconData {
    let size = 32usize;
    let mut rgba = vec![0u8; size * size * 4];
    let center = (size as f32 - 1.0) * 0.5;
    let radius = 14.5_f32;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let idx = (y * size + x) * 4;

            if dist <= radius {
                // Graphite circle background
                rgba[idx] = 24;
                rgba[idx + 1] = 26;
                rgba[idx + 2] = 31;
                rgba[idx + 3] = 255;

                // Inner accent dot
                if dist <= 6.2 {
                    rgba[idx] = 108;
                    rgba[idx + 1] = 140;
                    rgba[idx + 2] = 255;
                    rgba[idx + 3] = 255;
                }
            }
        }
    }

    egui::IconData {
        rgba,
        width: size as u32,
        height: size as u32,
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
