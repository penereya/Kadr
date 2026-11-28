use crate::audio::{self, AudioEditor};
use crate::media;
use crate::photo::{self, PhotoEditor};
use crate::theme;
use crate::updater;
use crate::video::{self, VideoCanvasAction, VideoEditor};
use eframe::egui::{self, Align, Color32, Layout, Margin, Pos2, Rect, Rounding, Stroke, Vec2};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Photo,
    Video,
    Audio,
}

impl ActiveTab {
    fn label(&self) -> &'static str {
        match self {
            ActiveTab::Photo => "Фото",
            ActiveTab::Video => "Видео",
            ActiveTab::Audio => "Аудио",
        }
    }
}

enum StatusKind {
    Success(Option<PathBuf>),
    Error,
}

struct StatusBanner {
    kind: StatusKind,
    message: String,
}

struct ExportOutcome {
    saved_path: PathBuf,
    deleted_original: Option<PathBuf>,
    warning: Option<String>,
}

pub struct KadrApp {
    tab: ActiveTab,
    photo: PhotoEditor,
    video: VideoEditor,
    audio: AudioEditor,

    ui_scale: f32,
    scale_cooldown_frames: u8,
    ffmpeg_ok: bool,
    open_dialog: Option<media::PendingFileDialog>,
    export_rx: Option<Receiver<Result<ExportOutcome, String>>>,
    export_progress: Option<Arc<AtomicU32>>,
    export_label: Option<String>,
    status: Option<StatusBanner>,
    screenshot_out: Option<PathBuf>,
    screenshot_started: std::time::Instant,
    screenshot_sent: bool,
    logo_texture: Option<egui::TextureHandle>,
    update_check_rx: Option<(Receiver<Result<updater::ReleaseInfo, String>>, bool)>,
    available_update: Option<updater::ReleaseInfo>,
    update_modal_open: bool,
    update_install_rx: Option<Receiver<Result<(), String>>>,
}

fn settings_file_path() -> Option<PathBuf> {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let dir = PathBuf::from(appdata).join("Kadr");
        return Some(dir.join("settings.txt"));
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("kadr_settings.txt")))
}

fn load_ui_scale() -> f32 {
    let Some(path) = settings_file_path() else {
        return 1.0;
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return 1.0;
    };
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("scale=") {
            if let Ok(s) = val.trim().parse::<f32>() {
                return s.clamp(0.8, 1.6);
            }
        }
    }
    1.0
}

fn save_ui_scale(scale: f32) {
    let Some(path) = settings_file_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, format!("scale={:.2}\n", scale.clamp(0.8, 1.6)));
}

fn load_brand_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let bytes = include_bytes!("../assets/logo.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color_img = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture("kadr_brand_logo", color_img, egui::TextureOptions::LINEAR))
}

impl KadrApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        updater::cleanup_old_binary();
        theme::setup_fonts(&cc.egui_ctx);
        theme::apply_graphite_theme(&cc.egui_ctx);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let saved_scale = load_ui_scale();
        cc.egui_ctx.set_zoom_factor(saved_scale);
        let logo_texture = load_brand_logo(&cc.egui_ctx);

        let mut app = Self {
            tab: ActiveTab::Photo,
            photo: PhotoEditor::default(),
            video: VideoEditor::default(),
            audio: AudioEditor::default(),
            ui_scale: saved_scale,
            scale_cooldown_frames: 0,
            ffmpeg_ok: media::check_ffmpeg_available(),
            open_dialog: None,
            export_rx: None,
            export_progress: None,
            export_label: None,
            status: None,
            screenshot_out: None,
            screenshot_started: std::time::Instant::now(),
            screenshot_sent: false,
            logo_texture,
            update_check_rx: None,
            available_update: None,
            update_modal_open: false,
            update_install_rx: None,
        };

        let mut args = std::env::args_os().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--screenshot" {
                if let Some(out) = args.next() {
                    app.screenshot_out = Some(PathBuf::from(out));
                }
            } else {
                let path = PathBuf::from(arg);
                if path.is_file() {
                    app.open_file_path(path, &cc.egui_ctx);
                }
            }
        }

        if app.screenshot_out.is_none() {
            app.start_update_check(&cc.egui_ctx, false);
        }

        app
    }

    fn start_update_check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.update_check_rx.is_some() || self.update_install_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.update_check_rx = Some((rx, manual));
        let ctx_clone = ctx.clone();
        std::thread::spawn(move || {
            let res = updater::check_latest_release();
            let _ = tx.send(res);
            ctx_clone.request_repaint();
        });
    }

    fn start_update_install(&mut self, ctx: &egui::Context, url: String) {
        if self.update_install_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.update_install_rx = Some(rx);
        let ctx_clone = ctx.clone();
        std::thread::spawn(move || {
            let res = updater::download_and_install_update(&url);
            let _ = tx.send(res);
            ctx_clone.request_repaint();
        });
    }

    fn poll_updates(&mut self, ctx: &egui::Context) {
        if let Some((rx, manual)) = &self.update_check_rx {
            if let Ok(res) = rx.try_recv() {
                let manual = *manual;
                self.update_check_rx = None;
                match res {
                    Ok(rel) => {
                        if updater::is_newer_version(updater::CURRENT_VERSION, &rel.tag) {
                            self.available_update = Some(rel);
                            if manual {
                                self.update_modal_open = true;
                            }
                        } else if manual {
                            self.status = Some(StatusBanner {
                                kind: StatusKind::Success(None),
                                message: format!(
                                    "У тебя установлена последняя версия Kadr (v{})",
                                    updater::CURRENT_VERSION
                                ),
                            });
                        }
                    }
                    Err(err) => {
                        if manual {
                            self.status = Some(StatusBanner {
                                kind: StatusKind::Error,
                                message: err,
                            });
                        }
                    }
                }
            }
        }

        if let Some(rx) = &self.update_install_rx {
            if let Ok(res) = rx.try_recv() {
                self.update_install_rx = None;
                match res {
                    Ok(()) => {
                        self.video.stop_playback();
                        self.audio.stop_playback();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Err(err) => {
                        self.update_modal_open = false;
                        self.status = Some(StatusBanner {
                            kind: StatusKind::Error,
                            message: err,
                        });
                    }
                }
            }
        }
    }

    fn render_update_modal(&mut self, ctx: &egui::Context) {
        if !self.update_modal_open {
            return;
        }
        let Some(rel) = self.available_update.clone() else {
            self.update_modal_open = false;
            return;
        };

        let mut open = self.update_modal_open;
        let installing = self.update_install_rx.is_some();

        egui::Window::new("Обновление Kadr")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(420.0, 260.0))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(theme::BG_SURFACE)
                    .stroke(Stroke::new(1.0_f32, theme::BORDER))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(Margin::same(18.0)),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "Текущая версия: v{}  →  Новая: {}",
                        updater::CURRENT_VERSION,
                        rel.tag
                    ))
                    .size(13.5)
                    .color(theme::TEXT_PRIMARY)
                    .strong(),
                );

                if !rel.notes.is_empty() {
                    ui.add_space(10.0);
                    egui::Frame::none()
                        .fill(theme::BG_BASE)
                        .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::same(10.0))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(140.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(&rel.notes)
                                            .size(12.0)
                                            .color(theme::TEXT_SECONDARY),
                                    );
                                });
                        });
                }

                ui.add_space(14.0);

                if installing {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Скачивание новой версии...")
                                .size(13.0)
                                .color(theme::TEXT_PRIMARY),
                        );
                    });
                } else {
                    ui.horizontal(|ui| {
                        if let Some(url) = rel.download_url.clone() {
                            let btn = egui::Button::new(
                                egui::RichText::new("Обновить сейчас")
                                    .size(13.0)
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(theme::ACCENT)
                            .stroke(Stroke::NONE)
                            .rounding(Rounding::same(8.0))
                            .min_size(Vec2::new(140.0, 32.0));
                            if ui.add(btn).clicked() {
                                self.start_update_install(ctx, url);
                            }
                        }

                        if theme::subtle_button(ui, "Открыть на GitHub").clicked() {
                            updater::open_url(&rel.html_url);
                        }

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if theme::subtle_button(ui, "Позже").clicked() {
                                self.update_modal_open = false;
                            }
                        });
                    });
                }
            });

        if !open && !installing {
            self.update_modal_open = false;
        }
    }

    fn is_busy(&self) -> bool {
        self.export_rx.is_some()
    }

    fn detect_tab(path: &Path) -> Option<ActiveTab> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" => Some(ActiveTab::Photo),
            "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => Some(ActiveTab::Video),
            "mp3" | "wav" | "ogg" | "m4a" | "flac" | "aac" => Some(ActiveTab::Audio),
            _ => None,
        }
    }

    fn open_file_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.video.stop_playback();
        self.audio.stop_playback();

        let detected = Self::detect_tab(&path);
        // If user is on Audio tab and drops/opens a video, extract/edit its audio track right here
        let target_tab = if self.tab == ActiveTab::Audio && detected == Some(ActiveTab::Video) {
            ActiveTab::Audio
        } else {
            detected.unwrap_or(self.tab)
        };
        self.tab = target_tab;
        self.status = None;

        let res = match target_tab {
            ActiveTab::Photo => self.photo.load(path),
            ActiveTab::Video => self.video.load(path, ctx),
            ActiveTab::Audio => self.audio.load(path, ctx),
        };

        if let Err(err) = res {
            self.status = Some(StatusBanner {
                kind: StatusKind::Error,
                message: err,
            });
        }
    }

    fn pick_file_dialog(&mut self, ctx: &egui::Context) {
        if self.open_dialog.is_some() {
            return;
        }
        let mut dialog = rfd::FileDialog::new();
        dialog = match self.tab {
            ActiveTab::Photo => dialog
                .add_filter("Изображения", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                .add_filter(
                    "Все медиа",
                    &["png", "jpg", "jpeg", "webp", "mp4", "mov", "mkv", "mp3", "wav", "ogg"],
                ),
            ActiveTab::Video => dialog
                .add_filter("Видео", &["mp4", "mov", "mkv", "avi", "webm", "m4v"])
                .add_filter(
                    "Все медиа",
                    &["mp4", "mov", "mkv", "png", "jpg", "mp3", "wav"],
                ),
            ActiveTab::Audio => dialog
                .add_filter(
                    "Аудио и видео (звук)",
                    &[
                        "mp3", "wav", "ogg", "m4a", "flac", "aac", "mp4", "mov", "mkv", "avi",
                        "webm", "m4v",
                    ],
                )
                .add_filter(
                    "Все медиа",
                    &["mp3", "wav", "ogg", "m4a", "mp4", "png", "jpg"],
                ),
        };

        self.open_dialog = Some(media::PendingFileDialog::spawn(ctx, move || {
            dialog.pick_file()
        }));
    }

    fn poll_background_export(&mut self) {
        if let Some(rx) = &self.export_rx {
            if let Ok(result) = rx.try_recv() {
                self.export_rx = None;
                self.export_progress = None;
                self.export_label = None;
                match result {
                    Ok(outcome) => {
                        if let Some(deleted) = &outcome.deleted_original {
                            if self.video.path.as_ref() == Some(deleted) {
                                self.video.clear();
                            }
                        }

                        let name = outcome
                            .saved_path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("файл")
                            .to_string();

                        let message = if outcome.deleted_original.is_some() {
                            format!("Готово! Сохранено: {} (оригинал перемещён в корзину)", name)
                        } else if let Some(warn) = outcome.warning {
                            format!("Готово! Сохранено: {} — {}", name, warn)
                        } else {
                            format!("Готово! Сохранено: {}", name)
                        };

                        self.status = Some(StatusBanner {
                            kind: StatusKind::Success(Some(outcome.saved_path)),
                            message,
                        });
                    }
                    Err(err) => {
                        self.status = Some(StatusBanner {
                            kind: StatusKind::Error,
                            message: err,
                        });
                    }
                }
            }
        }
    }

    fn handle_zoom_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let prev = self.ui_scale;
        ctx.input(|i| {
            if i.modifiers.command {
                if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                    self.ui_scale = ((self.ui_scale + 0.1).min(1.6) * 10.0).round() / 10.0;
                } else if i.key_pressed(egui::Key::Minus) {
                    self.ui_scale = ((self.ui_scale - 0.1).max(0.8) * 10.0).round() / 10.0;
                } else if i.key_pressed(egui::Key::Num0) {
                    self.ui_scale = 1.0;
                }
            }
        });
        if (self.ui_scale - prev).abs() > 0.001 {
            save_ui_scale(self.ui_scale);
        }
    }

    fn handle_app_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let (open_file, save_file, close_file, undo, redo, tab1, tab2, tab3) = ctx.input(|i| {
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            (
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::O),
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::S),
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::W),
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                cmd && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
                cmd && i.key_pressed(egui::Key::Num1),
                cmd && i.key_pressed(egui::Key::Num2),
                cmd && i.key_pressed(egui::Key::Num3),
            )
        });

        if tab1 {
            self.video.stop_playback();
            self.audio.stop_playback();
            self.tab = ActiveTab::Photo;
        } else if tab2 {
            self.audio.stop_playback();
            self.tab = ActiveTab::Video;
        } else if tab3 {
            self.video.stop_playback();
            self.tab = ActiveTab::Audio;
        }

        if open_file {
            self.pick_file_dialog(ctx);
        }
        if save_file && !self.is_busy() {
            match self.tab {
                ActiveTab::Photo => self.photo.request_save(ctx),
                ActiveTab::Video => self.video.request_save(ctx),
                ActiveTab::Audio => self.audio.request_save(ctx),
            }
        }
        if close_file {
            match self.tab {
                ActiveTab::Photo => self.photo.clear(),
                ActiveTab::Video => self.video.clear(),
                ActiveTab::Audio => self.audio.clear(),
            }
        }
        if undo {
            match self.tab {
                ActiveTab::Photo => {
                    self.photo.undo();
                }
                ActiveTab::Video => {
                    self.video.undo();
                }
                ActiveTab::Audio => {
                    self.audio.undo();
                }
            }
        }
        if redo {
            match self.tab {
                ActiveTab::Photo => {
                    self.photo.redo();
                }
                ActiveTab::Video => {
                    self.video.redo();
                }
                ActiveTab::Audio => {
                    self.audio.redo();
                }
            }
        }
    }

    fn handle_editor_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let reset_edits = ctx.input(|i| {
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            cmd && !i.modifiers.shift && i.key_pressed(egui::Key::R)
        });

        match self.tab {
            ActiveTab::Photo if self.photo.has_file() => {
                if reset_edits {
                    self.photo.reset_edits();
                }
                self.photo.handle_shortcuts(ctx);
            }
            ActiveTab::Video if self.video.has_file() => {
                if reset_edits {
                    self.video.reset_edits(ctx);
                }
                self.video.handle_shortcuts(ctx);
            }
            ActiveTab::Audio if self.audio.has_file() => {
                if reset_edits {
                    self.audio.reset_edits();
                }
                self.audio.handle_shortcuts(ctx);
            }
            _ => {}
        }
    }

    fn render_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar")
            .exact_height(58.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::BG_SURFACE)
                    .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                    .inner_margin(Margin::symmetric(18.0, 10.0)),
            )
            .show(ctx, |ui| {
                let bar_rect = ui.max_rect();

                let compact = bar_rect.width() < 640.0;
                let mut left_end = bar_rect.left() + 80.0;
                let mut right_start = bar_rect.right() - 250.0;

                ui.horizontal_centered(|ui| {
                    // Left: brand logo + KADR
                    let logo_sz = if compact { 28.0 } else { 32.0 };
                    if let Some(tex) = &self.logo_texture {
                        ui.add(egui::Image::new(tex).fit_to_exact_size(Vec2::splat(logo_sz)));
                    } else {
                        let (dot_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                        ui.painter()
                            .circle_filled(dot_rect.center(), 4.5, theme::ACCENT);
                    }
                    ui.add_space(6.0);
                    let brand_resp = ui.label(
                        egui::RichText::new("KADR")
                            .size(if compact { 14.5 } else { 16.0 })
                            .color(theme::TEXT_PRIMARY)
                            .strong(),
                    );
                    ui.add_space(6.0);
                    let ver_resp = if self.update_install_rx.is_some() {
                        ui.add(
                            egui::Button::new(
                                egui::RichText::new("Обновление...")
                                    .size(11.5)
                                    .color(theme::TEXT_PRIMARY),
                            )
                            .fill(theme::ACCENT_SOFT)
                            .stroke(Stroke::new(1.0_f32, theme::ACCENT))
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(60.0, 22.0)),
                        )
                    } else if let Some(rel) = self.available_update.clone() {
                        let label = if compact {
                            format!("↑ {}", rel.tag)
                        } else {
                            format!("Обновить {}", rel.tag)
                        };
                        let resp = ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(label)
                                        .size(11.5)
                                        .color(theme::TEXT_PRIMARY)
                                        .strong(),
                                )
                                .fill(theme::ACCENT_SOFT)
                                .stroke(Stroke::new(1.0_f32, theme::ACCENT))
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(52.0, 22.0)),
                            )
                            .on_hover_text("Доступна новая версия — нажми для обновления");
                        if resp.clicked() {
                            self.update_modal_open = true;
                        }
                        resp
                    } else if self.update_check_rx.is_some() {
                        ui.add(
                            egui::Button::new(
                                egui::RichText::new("Проверка...")
                                    .size(11.5)
                                    .color(theme::TEXT_MUTED),
                            )
                            .fill(theme::BG_BASE)
                            .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(52.0, 22.0)),
                        )
                    } else {
                        let resp = ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!("v{}", updater::CURRENT_VERSION))
                                        .size(11.5)
                                        .color(theme::TEXT_SECONDARY),
                                )
                                .fill(theme::BG_BASE)
                                .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(42.0, 22.0)),
                            )
                            .on_hover_text(
                                "Версия программы · Нажми, чтобы проверить обновления на GitHub",
                            );
                        if resp.clicked() {
                            self.start_update_check(ctx, true);
                        }
                        resp
                    };
                    left_end = brand_resp.rect.right().max(ver_resp.rect.right());

                    // Right: Open file button + UI scale capsule
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::subtle_button(ui, "Открыть файл").clicked()
                            && self.scale_cooldown_frames == 0
                        {
                            self.pick_file_dialog(ctx);
                        }

                        ui.add_space(if compact { 6.0 } else { 10.0 });

                        let zoom_resp = egui::Frame::none()
                            .fill(theme::BG_BASE)
                            .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                            .rounding(Rounding::same(8.0))
                            .inner_margin(Margin::symmetric(if compact { 2.0 } else { 4.0 }, 2.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = if compact { 0.0 } else { 2.0 };
                                    ui.spacing_mut().button_padding =
                                        Vec2::new(if compact { 4.0 } else { 8.0 }, 4.0);
                                    let zoom_btn = |ui: &mut egui::Ui, label: &str, min_w: f32| {
                                        ui.add(
                                            egui::Button::new(
                                                egui::RichText::new(label)
                                                    .size(if compact { 11.5 } else { 12.0 })
                                                    .color(theme::TEXT_PRIMARY),
                                            )
                                            .fill(Color32::TRANSPARENT)
                                            .stroke(Stroke::NONE)
                                            .rounding(Rounding::same(6.0))
                                            .min_size(Vec2::new(min_w, 24.0)),
                                        )
                                    };

                                    let pm_w = if compact { 18.0 } else { 24.0 };
                                    let pct_w = if compact { 36.0 } else { 44.0 };

                                    if zoom_btn(ui, "−", pm_w)
                                        .on_hover_text("Уменьшить интерфейс (Ctrl + −)")
                                        .clicked()
                                    {
                                        self.ui_scale =
                                            ((self.ui_scale - 0.1).max(0.8) * 10.0).round() / 10.0;
                                        self.scale_cooldown_frames = 3;
                                        save_ui_scale(self.ui_scale);
                                    }
                                    if zoom_btn(
                                        ui,
                                        &format!("{:.0}%", self.ui_scale * 100.0),
                                        pct_w,
                                    )
                                    .on_hover_text(
                                        "Масштаб интерфейса (клик — сброс на 100%, Ctrl + / −)",
                                    )
                                    .clicked()
                                    {
                                        self.ui_scale = 1.0;
                                        self.scale_cooldown_frames = 3;
                                        save_ui_scale(self.ui_scale);
                                    }
                                    if zoom_btn(ui, "+", pm_w)
                                        .on_hover_text("Увеличить интерфейс (Ctrl + +)")
                                        .clicked()
                                    {
                                        self.ui_scale =
                                            ((self.ui_scale + 0.1).min(1.6) * 10.0).round() / 10.0;
                                        self.scale_cooldown_frames = 3;
                                        save_ui_scale(self.ui_scale);
                                    }
                                });
                            });
                        right_start = zoom_resp.response.rect.left();
                    });
                });

                // Center: segmented tab pill centered within the remaining space without overlapping
                let pad = if compact { 6.0 } else { 10.0 };
                let avail_gap = (right_start - left_end - pad * 2.0).max(140.0);
                let tabs_w = 240.0_f32.min(avail_gap);
                let tab_btn_w = ((tabs_w - 14.0) / 3.0).clamp(42.0, 74.0);
                let min_cx = left_end + pad + tabs_w * 0.5;
                let max_cx = (right_start - pad - tabs_w * 0.5).max(min_cx);
                let center_x = bar_rect.center().x.clamp(min_cx, max_cx);
                let tabs_rect = Rect::from_center_size(
                    Pos2::new(center_x, bar_rect.center().y),
                    Vec2::new(tabs_w, 36.0),
                );

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {
                    ui.centered_and_justified(|ui| {
                        egui::Frame::none()
                            .fill(theme::BG_BASE)
                            .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                            .rounding(Rounding::same(10.0))
                            .inner_margin(Margin::same(3.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 3.0;
                                    ui.spacing_mut().button_padding =
                                        Vec2::new(if compact { 6.0 } else { 10.0 }, 4.0);
                                    for tab in
                                        [ActiveTab::Photo, ActiveTab::Video, ActiveTab::Audio]
                                    {
                                        let active = self.tab == tab;
                                        let fill = if active {
                                            theme::BG_ELEVATED
                                        } else {
                                            Color32::TRANSPARENT
                                        };
                                        let stroke = if active {
                                            Stroke::new(1.0_f32, theme::BORDER)
                                        } else {
                                            Stroke::NONE
                                        };
                                        let text_col = if active {
                                            theme::TEXT_PRIMARY
                                        } else {
                                            theme::TEXT_SECONDARY
                                        };

                                        let btn = egui::Button::new(
                                            egui::RichText::new(tab.label())
                                                .size(if compact { 12.0 } else { 13.0 })
                                                .color(text_col),
                                        )
                                        .fill(fill)
                                        .stroke(stroke)
                                        .rounding(Rounding::same(7.0))
                                        .min_size(Vec2::new(tab_btn_w, 28.0));

                                        if ui.add(btn).clicked() && self.tab != tab {
                                            self.video.stop_playback();
                                            self.audio.stop_playback();
                                            self.tab = tab;
                                        }
                                    }
                                });
                            });
                    });
                });
            });
    }

    fn render_status_bar(&mut self, ctx: &egui::Context) {
        if !self.is_busy() && self.status.is_none() {
            return;
        }

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(44.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::BG_SURFACE)
                    .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                    .inner_margin(Margin::symmetric(18.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if self.is_busy() {
                        ui.spinner();
                        ui.add_space(6.0);
                        let label = self.export_label.as_deref().unwrap_or("Обработка...");
                        if let Some(prog) = &self.export_progress {
                            let p = f32::from_bits(prog.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                            let pct = (p * 100.0).round() as u32;
                            ui.label(
                                egui::RichText::new(format!("{} {}%", label, pct))
                                    .size(13.0)
                                    .color(theme::TEXT_PRIMARY),
                            );
                            ui.add_space(12.0);
                            ui.add(
                                egui::ProgressBar::new(p)
                                    .fill(theme::ACCENT)
                                    .desired_width(240.0),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(label)
                                    .size(13.0)
                                    .color(theme::TEXT_PRIMARY),
                            );
                        }
                    } else if let Some(banner) = &self.status {
                        let (dot_col, reveal_path) = match &banner.kind {
                            StatusKind::Success(p) => (theme::SUCCESS, p.clone()),
                            StatusKind::Error => (theme::ERROR, None),
                        };

                        let (dot_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot_rect.center(), 4.0, dot_col);
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(&banner.message)
                                .size(13.0)
                                .color(theme::TEXT_PRIMARY),
                        );

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("✕")
                                            .size(12.0)
                                            .color(theme::TEXT_SECONDARY),
                                    )
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE),
                                )
                                .clicked()
                            {
                                self.status = None;
                            }

                            if let Some(p) = reveal_path {
                                if theme::subtle_button(ui, "Показать в папке").clicked() {
                                    media::reveal_in_explorer(&p);
                                }
                            }
                        });
                    }
                });
            });
    }

    fn render_empty_state(&mut self, ui: &mut egui::Ui) {
        let (title, formats) = match self.tab {
            ActiveTab::Photo => (
                "Перетащи фото сюда или выбери файл",
                "PNG, JPG, WEBP, BMP, GIF",
            ),
            ActiveTab::Video => (
                "Перетащи видео сюда или выбери файл",
                "MP4, MOV, MKV, WEBM, AVI",
            ),
            ActiveTab::Audio => (
                "Перетащи аудио или видео сюда",
                "MP3, WAV, OGG, M4A, FLAC  •  из видео достанет звук",
            ),
        };

        let avail = ui.available_rect_before_wrap();
        let card_h = if !self.ffmpeg_ok && self.tab != ActiveTab::Photo {
            282.0
        } else {
            252.0
        };
        let card_size = Vec2::new(400.0_f32.min(avail.width() - 24.0), card_h);
        let card_rect = Rect::from_center_size(avail.center(), card_size);

        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(card_rect), |ui| {
            egui::Frame::none()
                .fill(theme::BG_SURFACE)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .rounding(Rounding::same(18.0))
                .inner_margin(Margin::symmetric(36.0, 34.0))
                .show(ui, |ui| {
                    ui.set_min_size(Vec2::new(
                        (card_size.x - 72.0).max(120.0),
                        (card_size.y - 68.0).max(120.0),
                    ));
                    ui.vertical_centered(|ui| {
                        // Minimalist vector emblem
                        let (icon_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(56.0), egui::Sense::hover());
                        let p = ui.painter();
                        p.rect_filled(icon_rect, Rounding::same(14.0), theme::BG_ELEVATED);
                        p.rect_stroke(
                            icon_rect,
                            Rounding::same(14.0),
                            Stroke::new(1.0_f32, theme::BORDER),
                        );
                        let c = icon_rect.center();
                        match self.tab {
                            ActiveTab::Photo => {
                                p.rect_stroke(
                                    Rect::from_center_size(c, Vec2::new(24.0, 18.0)),
                                    Rounding::same(3.0),
                                    Stroke::new(1.6_f32, theme::ACCENT),
                                );
                                p.circle_filled(c + Vec2::new(-5.0, -3.0), 2.3, theme::ACCENT);
                                p.line_segment(
                                    [c + Vec2::new(-9.0, 6.0), c + Vec2::new(-1.0, 0.0)],
                                    Stroke::new(1.6_f32, theme::ACCENT),
                                );
                                p.line_segment(
                                    [c + Vec2::new(-1.0, 0.0), c + Vec2::new(8.0, 6.0)],
                                    Stroke::new(1.6_f32, theme::ACCENT),
                                );
                            }
                            ActiveTab::Video => {
                                let pts = vec![
                                    c + Vec2::new(-6.0, -9.0),
                                    c + Vec2::new(10.0, 0.0),
                                    c + Vec2::new(-6.0, 9.0),
                                ];
                                p.add(egui::Shape::convex_polygon(pts, theme::ACCENT, Stroke::NONE));
                            }
                            ActiveTab::Audio => {
                                let heights = [10.0, 22.0, 14.0, 26.0, 12.0];
                                for (i, h) in heights.iter().enumerate() {
                                    let x = c.x + (i as f32 - 2.0) * 6.0;
                                    let bar = Rect::from_center_size(
                                        Pos2::new(x, c.y),
                                        Vec2::new(3.0, *h),
                                    );
                                    p.rect_filled(bar, Rounding::same(1.5), theme::ACCENT);
                                }
                            }
                        }

                        ui.add_space(18.0);
                        ui.label(
                            egui::RichText::new(title)
                                .size(15.5)
                                .color(theme::TEXT_PRIMARY)
                                .strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(formats)
                                .size(12.5)
                                .color(theme::TEXT_MUTED),
                        );

                        if !self.ffmpeg_ok && self.tab != ActiveTab::Photo {
                            ui.add_space(10.0);
                            ui.label(
                                egui::RichText::new("Внимание: FFmpeg не найден в системе")
                                    .size(12.0)
                                    .color(theme::ERROR),
                            );
                        }

                        ui.add_space(20.0);
                        let btn = egui::Button::new(
                            egui::RichText::new("Выбрать файл")
                                .size(13.5)
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(theme::ACCENT)
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(9.0))
                        .min_size(Vec2::new(180.0, 38.0));

                        if ui.add(btn).clicked() {
                            self.pick_file_dialog(ui.ctx());
                        }
                    });
                });
        });
    }

    fn handle_drag_and_drop(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });

        if let Some(first) = dropped.into_iter().next() {
            self.open_file_path(first, ctx);
        }

        let is_hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if is_hovering {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_overlay"),
            ));
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(175));
            painter.rect_stroke(
                screen.shrink(24.0),
                Rounding::same(16.0),
                Stroke::new(2.0_f32, theme::ACCENT),
            );
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Отпусти файл, чтобы открыть",
                egui::FontId::proportional(20.0),
                theme::TEXT_PRIMARY,
            );
        }
    }
}

impl eframe::App for KadrApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.scale_cooldown_frames > 0 {
            self.scale_cooldown_frames -= 1;
        }
        self.handle_zoom_shortcuts(ctx);
        ctx.set_zoom_factor(self.ui_scale);

        if let Some(picked) = self.open_dialog.as_ref().and_then(|d| d.poll()) {
            self.open_dialog = None;
            if let Some(path) = picked {
                self.open_file_path(path, ctx);
            }
        }

        self.poll_background_export();
        self.poll_updates(ctx);
        if self.is_busy() || self.update_check_rx.is_some() || self.update_install_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }

        self.handle_app_shortcuts(ctx);
        self.render_update_modal(ctx);

        match self.tab {
            ActiveTab::Photo if self.photo.has_file() => self.photo.begin_history_frame(),
            ActiveTab::Video if self.video.has_file() => self.video.begin_history_frame(),
            ActiveTab::Audio if self.audio.has_file() => self.audio.begin_history_frame(),
            _ => {}
        }
        self.handle_editor_shortcuts(ctx);

        self.render_top_bar(ctx);
        self.render_status_bar(ctx);

        let has_active_file = match self.tab {
            ActiveTab::Photo => self.photo.has_file(),
            ActiveTab::Video => self.video.has_file(),
            ActiveTab::Audio => self.audio.has_file(),
        };

        if has_active_file {
            // Right inspector panel (adapts gracefully in compact/zoomed windows)
            let win_w = ctx.screen_rect().width();
            let inspector_w = (win_w * 0.44).clamp(245.0, 320.0);
            egui::SidePanel::right("inspector")
                .exact_width(inspector_w)
                .resizable(false)
                .frame(
                    egui::Frame::none()
                        .fill(theme::BG_BASE)
                        .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                        .inner_margin(Margin::same(14.0)),
                )
                .show(ctx, |ui| match self.tab {
                    ActiveTab::Photo => {
                        let busy = self.is_busy();
                        if let Some(job) = self.photo.render_inspector(ui, busy) {
                            let (tx, rx) = mpsc::channel();
                            self.export_rx = Some(rx);
                            self.export_label = Some("Сохранение фото...".to_string());
                            self.status = None;
                            let ctx_clone = ctx.clone();
                            std::thread::spawn(move || {
                                let res = photo::run_photo_export(&job).map(|saved_path| {
                                    ExportOutcome {
                                        saved_path,
                                        deleted_original: None,
                                        warning: None,
                                    }
                                });
                                let _ = tx.send(res);
                                ctx_clone.request_repaint();
                            });
                        }
                    }
                    ActiveTab::Video => {
                        let busy = self.is_busy();
                        if let Some(job) = self.video.render_inspector(ui, busy) {
                            let (tx, rx) = mpsc::channel();
                            let prog = Arc::new(AtomicU32::new(0.0_f32.to_bits()));
                            self.export_rx = Some(rx);
                            self.export_progress = Some(Arc::clone(&prog));
                            self.export_label = Some("Экспорт видео...".to_string());
                            self.status = None;
                            let ctx_clone = ctx.clone();
                            std::thread::spawn(move || {
                                let res = video::run_video_export(&job, prog).map(|r| ExportOutcome {
                                    saved_path: r.saved_path,
                                    deleted_original: r.deleted_original,
                                    warning: r.warning,
                                });
                                let _ = tx.send(res);
                                ctx_clone.request_repaint();
                            });
                        }
                    }
                    ActiveTab::Audio => {
                        let busy = self.is_busy();
                        if let Some(job) = self.audio.render_inspector(ui, busy) {
                            let (tx, rx) = mpsc::channel();
                            let prog = Arc::new(AtomicU32::new(0.0_f32.to_bits()));
                            self.export_rx = Some(rx);
                            self.export_progress = Some(Arc::clone(&prog));
                            self.export_label = Some("Экспорт аудио...".to_string());
                            self.status = None;
                            let ctx_clone = ctx.clone();
                            std::thread::spawn(move || {
                                let res = audio::run_audio_export(&job, prog).map(|saved_path| {
                                    ExportOutcome {
                                        saved_path,
                                        deleted_original: None,
                                        warning: None,
                                    }
                                });
                                let _ = tx.send(res);
                                ctx_clone.request_repaint();
                            });
                        }
                    }
                });
        }

        // Main center workspace
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(theme::BG_BASE)
                    .inner_margin(Margin::same(18.0)),
            )
            .show(ctx, |ui| {
                if !has_active_file {
                    self.render_empty_state(ui);
                } else {
                    match self.tab {
                        ActiveTab::Photo => self.photo.render_canvas(ui),
                        ActiveTab::Video => match self.video.render_canvas(ui) {
                            VideoCanvasAction::None => {}
                            VideoCanvasAction::Deleted(name) => {
                                self.status = Some(StatusBanner {
                                    kind: StatusKind::Success(None),
                                    message: format!("Видео «{}» перемещено в корзину", name),
                                });
                            }
                            VideoCanvasAction::DeleteError(err) => {
                                self.status = Some(StatusBanner {
                                    kind: StatusKind::Error,
                                    message: err,
                                });
                            }
                        },
                        ActiveTab::Audio => self.audio.render_canvas(ui),
                    }
                }
            });

        let pointer_down = ctx.input(|i| i.pointer.any_down());
        match self.tab {
            ActiveTab::Photo if self.photo.has_file() => self.photo.end_history_frame(pointer_down),
            ActiveTab::Video if self.video.has_file() => self.video.end_history_frame(pointer_down),
            ActiveTab::Audio if self.audio.has_file() => self.audio.end_history_frame(pointer_down),
            _ => {}
        }

        self.handle_drag_and_drop(ctx);

        if let Some(out_path) = &self.screenshot_out {
            let captured = ctx.input(|i| {
                for ev in &i.raw.events {
                    if let egui::Event::Screenshot { image, .. } = ev {
                        return Some(Arc::clone(image));
                    }
                }
                None
            });

            if let Some(img) = captured {
                if let Some(parent) = out_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = image::save_buffer(
                    out_path,
                    img.as_raw(),
                    img.width() as u32,
                    img.height() as u32,
                    image::ColorType::Rgba8,
                );
                self.screenshot_out = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if !self.screenshot_sent {
                if self.screenshot_started.elapsed() >= std::time::Duration::from_millis(1200) {
                    self.screenshot_sent = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.video.stop_playback();
        self.audio.stop_playback();
    }
}
