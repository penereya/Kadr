use crate::media::{self, MediaInfo};
use crate::theme;
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Rounding, Stroke, Vec2};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct AudioPlayer {
    stop_flag: Arc<AtomicBool>,
    volume_bits: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    started_at: Instant,
    start_time: f64,
    end_time: f64,
}

impl AudioPlayer {
    fn start(path: &Path, start_time: f64, end_time: f64, volume_pct: f32) -> Option<Self> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let volume_bits = Arc::new(AtomicU32::new(volume_pct.to_bits()));
        let muted = Arc::new(AtomicBool::new(false));
        let duration = (end_time - start_time).max(0.1);
        let child = media::spawn_live_audio_stream(
            path.to_path_buf(),
            start_time.max(0.0),
            duration,
            Arc::clone(&volume_bits),
            Arc::clone(&muted),
            Arc::clone(&stop_flag),
        )?;
        Some(Self {
            stop_flag,
            volume_bits,
            muted,
            child,
            started_at: Instant::now(),
            start_time,
            end_time,
        })
    }

    fn stop(self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Mp3,
    Wav,
    Ogg,
    M4a,
}

impl AudioFormat {
    pub fn ext(&self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Wav => "wav",
            AudioFormat::Ogg => "ogg",
            AudioFormat::M4a => "m4a",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "MP3",
            AudioFormat::Wav => "WAV",
            AudioFormat::Ogg => "OGG",
            AudioFormat::M4a => "M4A",
        }
    }
}

#[derive(Clone)]
pub struct AudioExportJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub trim_start: f64,
    pub trim_end: f64,
    pub volume_pct: f32,
    pub fade_in: f64,
    pub fade_out: f64,
    pub format: AudioFormat,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AudioTimelineDrag {
    None,
    Playhead,
    TrimStart,
    TrimEnd,
    TrimWindow,
}

pub struct AudioEditor {
    pub path: Option<PathBuf>,
    pub info: MediaInfo,
    pub waveform: Vec<f32>,

    pub trim_start: f64,
    pub trim_end: f64,
    pub volume_pct: f32, // 0.0..=300.0 (100.0 = original)
    pub fade_in: f64,    // seconds
    pub fade_out: f64,   // seconds
    pub format: AudioFormat,
    save_dialog: Option<media::PendingFileDialog>,
    history: media::UndoHistory<AudioSnapshot>,
    frame_snapshot: Option<AudioSnapshot>,
    player: Option<AudioPlayer>,
    play_time: f64,
    active_drag: AudioTimelineDrag,
    drag_window_offset_secs: f64,
    resume_after_drag: bool,
}

#[derive(Clone, PartialEq)]
pub struct AudioSnapshot {
    trim_start: f64,
    trim_end: f64,
    volume_pct: f32,
    fade_in: f64,
    fade_out: f64,
    format: AudioFormat,
}

impl Default for AudioEditor {
    fn default() -> Self {
        Self {
            path: None,
            info: MediaInfo::default(),
            waveform: Vec::new(),
            trim_start: 0.0,
            trim_end: 1.0,
            volume_pct: 15.0,
            fade_in: 0.0,
            fade_out: 0.0,
            format: AudioFormat::Mp3,
            save_dialog: None,
            history: media::UndoHistory::default(),
            frame_snapshot: None,
            player: None,
            play_time: 0.0,
            active_drag: AudioTimelineDrag::None,
            drag_window_offset_secs: 0.0,
            resume_after_drag: false,
        }
    }
}

impl AudioEditor {
    pub fn has_file(&self) -> bool {
        self.path.is_some()
    }

    pub fn is_playing(&self) -> bool {
        self.player.is_some()
    }

    pub fn stop_playback(&mut self) {
        if let Some(p) = self.player.take() {
            p.stop();
        }
    }

    fn effective_playback_volume(&self) -> f32 {
        let mut mult = 1.0_f32;
        if self.play_time >= self.trim_start && self.play_time <= self.trim_end {
            if self.fade_in > 0.05 && (self.play_time - self.trim_start) < self.fade_in {
                mult *= ((self.play_time - self.trim_start) / self.fade_in).clamp(0.0, 1.0) as f32;
            }
            if self.fade_out > 0.05 && (self.trim_end - self.play_time) < self.fade_out {
                mult *= ((self.trim_end - self.play_time) / self.fade_out).clamp(0.0, 1.0) as f32;
            }
        }
        self.volume_pct * mult
    }

    fn sync_live_audio(&self) {
        if let Some(p) = &self.player {
            let eff = self.effective_playback_volume();
            p.volume_bits.store(eff.to_bits(), Ordering::Relaxed);
            p.muted.store(eff <= 0.1, Ordering::Relaxed);
        }
    }

    fn start_playback_at(&mut self, start: f64) {
        self.stop_playback();
        let Some(path) = self.path.clone() else { return };
        let dur = self.info.duration_secs.max(0.1);
        let clamped_start = start.clamp(0.0, (dur - 0.1).max(0.0));
        self.play_time = clamped_start;
        let end = if clamped_start < self.trim_end - 0.05 {
            self.trim_end.min(dur)
        } else {
            dur
        };
        let init_vol = self.effective_playback_volume();
        if let Some(p) =
            AudioPlayer::start(&path, clamped_start, end.max(clamped_start + 0.1), init_vol)
        {
            self.player = Some(p);
        }
    }

    pub fn toggle_playback(&mut self) {
        if self.is_playing() {
            self.stop_playback();
            return;
        }
        let start = if self.play_time >= self.trim_end - 0.05
            && self.play_time <= self.trim_end + 0.15
        {
            self.trim_start
        } else {
            self.play_time
        };
        self.start_playback_at(start);
    }

    pub fn load(&mut self, path: PathBuf) -> Result<(), String> {
        self.stop_playback();
        let info = media::probe_media(&path)?;
        let waveform = media::extract_waveform(&path, 140).unwrap_or_else(|_| vec![0.25; 140]);
        self.info = info.clone();
        self.waveform = waveform;
        self.path = Some(path);
        self.reset_edits();
        self.volume_pct = 15.0;
        self.play_time = 0.0;
        self.active_drag = AudioTimelineDrag::None;
        self.history.clear();
        self.frame_snapshot = None;
        Ok(())
    }

    pub fn clear(&mut self) {
        self.stop_playback();
        self.path = None;
        self.info = MediaInfo::default();
        self.waveform.clear();
        self.play_time = 0.0;
        self.active_drag = AudioTimelineDrag::None;
        self.resume_after_drag = false;
        self.history.clear();
        self.frame_snapshot = None;
    }

    pub fn snapshot(&self) -> AudioSnapshot {
        AudioSnapshot {
            trim_start: self.trim_start,
            trim_end: self.trim_end,
            volume_pct: self.volume_pct,
            fade_in: self.fade_in,
            fade_out: self.fade_out,
            format: self.format,
        }
    }

    pub fn apply_snapshot(&mut self, s: AudioSnapshot) {
        self.trim_start = s.trim_start;
        self.trim_end = s.trim_end;
        self.volume_pct = s.volume_pct;
        self.fade_in = s.fade_in;
        self.fade_out = s.fade_out;
        self.format = s.format;
        self.sync_live_audio();
    }

    pub fn begin_history_frame(&mut self) {
        self.frame_snapshot = Some(self.snapshot());
    }

    pub fn end_history_frame(&mut self, pointer_down: bool) {
        if let Some(before) = self.frame_snapshot.take() {
            let after = self.snapshot();
            self.history.record_frame(&before, &after, pointer_down);
        }
    }

    pub fn undo(&mut self) -> bool {
        let cur = self.snapshot();
        if let Some(prev) = self.history.undo(&cur) {
            self.apply_snapshot(prev);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        let cur = self.snapshot();
        if let Some(next) = self.history.redo(&cur) {
            self.apply_snapshot(next);
            true
        } else {
            false
        }
    }

    pub fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let (toggle, seek_delta, go_home, shift_home, go_end, set_in, set_out, vol_delta, mute) =
            ctx.input(|i| {
                let no_mods = !i.modifiers.ctrl && !i.modifiers.command && !i.modifiers.alt;
                let step = if i.modifiers.shift { 0.5 } else { 5.0 };
                let mut seek = 0.0;
                if no_mods && i.key_pressed(egui::Key::ArrowLeft) {
                    seek -= step;
                }
                if no_mods && i.key_pressed(egui::Key::ArrowRight) {
                    seek += step;
                }
                let mut vol = 0.0_f32;
                if no_mods && i.key_pressed(egui::Key::ArrowUp) {
                    vol += 5.0;
                }
                if no_mods && i.key_pressed(egui::Key::ArrowDown) {
                    vol -= 5.0;
                }
                (
                    no_mods && i.key_pressed(egui::Key::Space),
                    seek,
                    no_mods && i.key_pressed(egui::Key::Home),
                    i.modifiers.shift,
                    no_mods && i.key_pressed(egui::Key::End),
                    no_mods && i.key_pressed(egui::Key::I),
                    no_mods && i.key_pressed(egui::Key::O),
                    vol,
                    no_mods && i.key_pressed(egui::Key::M),
                )
            });

        if toggle {
            self.toggle_playback();
        }
        if seek_delta != 0.0 {
            let dur = self.info.duration_secs.max(0.1);
            let target = (self.play_time + seek_delta).clamp(0.0, dur);
            self.play_time = target;
            if self.is_playing() {
                self.start_playback_at(target);
            }
        }
        if go_home {
            let target = if shift_home { 0.0 } else { self.trim_start };
            self.play_time = target;
            if self.is_playing() {
                self.start_playback_at(target);
            }
        }
        if go_end {
            let target = self.trim_end;
            self.play_time = target;
            if self.is_playing() {
                self.start_playback_at(target);
            }
        }
        if set_in {
            self.trim_start = self.play_time.clamp(0.0, (self.trim_end - 0.1).max(0.0));
        }
        if set_out {
            let dur = self.info.duration_secs.max(0.1);
            self.trim_end = self.play_time.clamp((self.trim_start + 0.1).min(dur), dur);
        }
        if vol_delta != 0.0 {
            self.volume_pct = (self.volume_pct + vol_delta).clamp(0.0, 300.0);
            self.sync_live_audio();
        }
        if mute {
            self.volume_pct = if self.volume_pct > 0.5 { 0.0 } else { 15.0 };
            self.sync_live_audio();
        }
    }

    pub fn request_save(&mut self, ctx: &egui::Context) {
        if !self.has_file() || self.save_dialog.is_some() {
            return;
        }
        self.stop_playback();
        let ext = self.format.ext();
        let label = self.format.label();
        let default_name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(|s| format!("{}_edited.{}", s, ext))
            .unwrap_or_else(|| format!("audio_edited.{}", ext));

        self.save_dialog = Some(media::PendingFileDialog::spawn(ctx, move || {
            rfd::FileDialog::new()
                .set_file_name(&default_name)
                .add_filter(label, &[ext])
                .save_file()
        }));
    }

    pub fn reset_edits(&mut self) {
        self.trim_start = 0.0;
        self.trim_end = self.info.duration_secs.max(0.1);
        self.volume_pct = 100.0;
        self.fade_in = 0.0;
        self.fade_out = 0.0;
        self.format = AudioFormat::Mp3;
    }

    pub fn render_canvas(&mut self, ui: &mut egui::Ui) {
        // Update playback clock
        let mut finished_playback = false;
        if let Some(p) = &self.player {
            let elapsed = p.started_at.elapsed().as_secs_f64();
            let cur = p.start_time + elapsed;
            if cur >= p.end_time {
                self.play_time = p.end_time;
                finished_playback = true;
            } else {
                self.play_time = cur;
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
        }
        if finished_playback {
            self.stop_playback();
        } else if self.is_playing() {
            self.sync_live_audio();
        }

        // Top metadata bar
        let compact_bar = ui.available_width() < 420.0;
        ui.horizontal_wrapped(|ui| {
            if let Some(p) = &self.path {
                let full_name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Аудиофайл");
                let display_name = if compact_bar && full_name.chars().count() > 20 {
                    let head: String = full_name.chars().take(19).collect();
                    format!("{}…", head)
                } else {
                    full_name.to_string()
                };
                ui.label(
                    egui::RichText::new(display_name)
                        .size(13.5)
                        .color(theme::TEXT_PRIMARY)
                        .strong(),
                )
                .on_hover_text(full_name);

                if theme::subtle_button(ui, "Закрыть")
                    .on_hover_text("Закрыть файл без удаления")
                    .clicked()
                {
                    self.clear();
                    return;
                }
            }
            if ui.max_rect().right() - ui.cursor().left() >= 200.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::metadata_pill(ui, &theme::format_bytes(self.info.file_size));
                    if self.info.bitrate_kbps > 0 {
                        theme::metadata_pill(ui, &format!("{} кбит/с", self.info.bitrate_kbps));
                    }
                    theme::metadata_pill(ui, &theme::format_time(self.info.duration_secs));
                });
            }
        });

        if !self.has_file() {
            return;
        }

        ui.add_space(10.0);

        // Main waveform canvas (leave room for playback + two-zone trim bar below)
        let bar_h_total = if ui.available_height() < 320.0 { 108.0 } else { 118.0 };
        let canvas_h = (ui.available_height() - bar_h_total - 10.0).max(80.0);
        let avail = Vec2::new(ui.available_width(), canvas_h);
        egui::Frame::none()
            .fill(Color32::from_rgb(14, 14, 15))
            .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
            .rounding(Rounding::same(14.0))
            .show(ui, |ui| {
                ui.set_min_size(avail);
                let (rect, _) = ui.allocate_exact_size(avail, egui::Sense::hover());

                let dur = self.info.duration_secs.max(0.1);
                let clip_len = (self.trim_end - self.trim_start).max(0.0);

                // Inner waveform area
                let wave_rect = Rect::from_center_size(
                    rect.center(),
                    Vec2::new((rect.width() - 56.0).max(120.0), (rect.height() * 0.52).clamp(90.0, 260.0)),
                );

                // Click/drag anywhere on waveform to seek playhead
                let wave_resp = ui.interact(
                    wave_rect.expand2(Vec2::new(4.0, 14.0)),
                    ui.id().with("audio_wave_seek"),
                    egui::Sense::click_and_drag(),
                );
                if wave_resp.clicked() || wave_resp.dragged() {
                    if let Some(pos) = wave_resp.interact_pointer_pos() {
                        let ratio = ((pos.x - wave_rect.left()) / wave_rect.width().max(1.0))
                            .clamp(0.0, 1.0) as f64;
                        self.play_time = ratio * dur;
                        if wave_resp.clicked() && self.is_playing() {
                            self.start_playback_at(self.play_time);
                        }
                    }
                }
                if wave_resp.drag_stopped() && self.is_playing() {
                    self.start_playback_at(self.play_time);
                }

                let p = ui.painter();

                // Subtle horizontal center line
                p.line_segment(
                    [
                        Pos2::new(wave_rect.left(), wave_rect.center().y),
                        Pos2::new(wave_rect.right(), wave_rect.center().y),
                    ],
                    Stroke::new(1.0, theme::BORDER_SUBTLE),
                );

                let start_ratio = (self.trim_start / dur) as f32;
                let end_ratio = (self.trim_end / dur) as f32;

                // Highlighted active trim backdrop
                let sel_left = wave_rect.left() + wave_rect.width() * start_ratio;
                let sel_right = wave_rect.left() + wave_rect.width() * end_ratio;
                let sel_rect = Rect::from_min_max(
                    Pos2::new(sel_left, wave_rect.top() - 10.0),
                    Pos2::new(sel_right.max(sel_left + 4.0), wave_rect.bottom() + 10.0),
                );
                p.rect_filled(sel_rect, Rounding::same(8.0), Color32::from_rgba_unmultiplied(217, 83, 47, 34));

                // Draw waveform bars
                let count = self.waveform.len().max(1);
                let step = wave_rect.width() / count as f32;
                let bar_w = (step * 0.62).clamp(2.0, 6.0);
                let vol_scale = (self.volume_pct / 100.0).clamp(0.1, 1.5);

                for (i, &amp) in self.waveform.iter().enumerate() {
                    let t_ratio = (i as f32 + 0.5) / count as f32;
                    let t_sec = t_ratio as f64 * dur;
                    let in_range = t_sec >= self.trim_start && t_sec <= self.trim_end;

                    // Compute fade envelope multiplier for visual feedback
                    let mut fade_mult = 1.0_f32;
                    if in_range {
                        if self.fade_in > 0.05 && (t_sec - self.trim_start) < self.fade_in {
                            fade_mult *= ((t_sec - self.trim_start) / self.fade_in).clamp(0.05, 1.0) as f32;
                        }
                        if self.fade_out > 0.05 && (self.trim_end - t_sec) < self.fade_out {
                            fade_mult *= ((self.trim_end - t_sec) / self.fade_out).clamp(0.05, 1.0) as f32;
                        }
                    }

                    let visual_amp = if in_range {
                        (amp * vol_scale * fade_mult).clamp(0.04, 1.0)
                    } else {
                        amp.clamp(0.04, 1.0)
                    };

                    let bar_h = (wave_rect.height() * visual_amp).max(4.0);
                    let cx = wave_rect.left() + step * (i as f32 + 0.5);
                    let bar_rect = Rect::from_center_size(
                        Pos2::new(cx, wave_rect.center().y),
                        Vec2::new(bar_w, bar_h),
                    );

                    let color = if in_range {
                        theme::ACCENT
                    } else {
                        Color32::from_rgb(56, 55, 54)
                    };
                    p.rect_filled(bar_rect, Rounding::same(bar_w * 0.5), color);
                }

                // Trim boundary lines
                p.line_segment(
                    [Pos2::new(sel_left, sel_rect.top()), Pos2::new(sel_left, sel_rect.bottom())],
                    Stroke::new(2.0, theme::ACCENT_HOVER),
                );
                p.line_segment(
                    [Pos2::new(sel_right, sel_rect.top()), Pos2::new(sel_right, sel_rect.bottom())],
                    Stroke::new(2.0, theme::ACCENT_HOVER),
                );

                // Playhead line + cap
                let play_ratio = (self.play_time / dur).clamp(0.0, 1.0) as f32;
                let play_x = wave_rect.left() + wave_rect.width() * play_ratio;
                p.line_segment(
                    [
                        Pos2::new(play_x, sel_rect.top() - 4.0),
                        Pos2::new(play_x, sel_rect.bottom() + 4.0),
                    ],
                    Stroke::new(2.0_f32, Color32::WHITE),
                );
                p.circle_filled(Pos2::new(play_x, sel_rect.top() - 4.0), 4.5, Color32::WHITE);

                // Bottom summary text inside canvas
                let summary = format!(
                    "Выбранный фрагмент: {} — {}   •   Длительность: {}   •   Громкость: {:.0}%",
                    theme::format_time(self.trim_start),
                    theme::format_time(self.trim_end),
                    theme::format_time(clip_len),
                    self.volume_pct
                );
                p.text(
                    Pos2::new(rect.center().x, wave_rect.bottom() + 34.0),
                    egui::Align2::CENTER_CENTER,
                    summary,
                    egui::FontId::proportional(12.5),
                    theme::TEXT_SECONDARY,
                );
            });

        ui.add_space(10.0);

        // Bottom playback control bar
        egui::Frame::none()
            .fill(theme::BG_SURFACE)
            .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
            .rounding(Rounding::same(12.0))
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                let dur = self.info.duration_secs.max(0.1);
                ui.horizontal(|ui| {
                    let play_label = if self.is_playing() {
                        "⏸  Пауза"
                    } else {
                        "▶  Слушать"
                    };
                    let play_btn = egui::Button::new(
                        egui::RichText::new(play_label)
                            .size(12.5)
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(theme::ACCENT)
                    .stroke(Stroke::NONE)
                    .rounding(Rounding::same(7.0))
                    .min_size(Vec2::new(104.0, 28.0));

                    if ui.add(play_btn).clicked() {
                        self.toggle_playback();
                    }

                    let rewind_btn = egui::Button::new(
                        egui::RichText::new("⏮  В начало")
                            .size(12.0)
                            .color(theme::TEXT_PRIMARY),
                    )
                    .fill(theme::BG_ELEVATED)
                    .stroke(Stroke::new(1.0_f32, theme::BORDER))
                    .rounding(Rounding::same(7.0))
                    .min_size(Vec2::new(82.0, 28.0));

                    if ui
                        .add(rewind_btn)
                        .on_hover_text("В начало фрагмента (Shift — в самое начало 00:00)")
                        .clicked()
                    {
                        let to_zero = ui.ctx().input(|i| i.modifiers.shift);
                        let target = if to_zero { 0.0 } else { self.trim_start };
                        self.play_time = target;
                        if self.is_playing() {
                            self.start_playback_at(target);
                        }
                    }

                    let clip_len = (self.trim_end - self.trim_start).max(0.0);
                    let rem_after_btn = ui.max_rect().right() - ui.cursor().left();
                    if rem_after_btn >= 300.0 {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "Фрагмент: {} — {}  (итого {})",
                                theme::format_time(self.trim_start),
                                theme::format_time(self.trim_end),
                                theme::format_time(clip_len),
                            ))
                            .size(12.0)
                            .color(theme::TEXT_SECONDARY),
                        );
                    } else if rem_after_btn >= 180.0 {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("Итого: {}", theme::format_time(clip_len)))
                                .size(11.5)
                                .color(theme::TEXT_SECONDARY),
                        );
                    }

                    if ui.max_rect().right() - ui.cursor().left() >= 100.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} / {}",
                                    theme::format_time(self.play_time),
                                    theme::format_time(dur)
                                ))
                                .size(12.0)
                                .color(theme::TEXT_PRIMARY),
                            );
                        });
                    }
                });

                ui.add_space(8.0);

                // Two-zone draggable timeline bar (top ruler for white playhead, bottom track for trim handles)
                let bar_size = Vec2::new(ui.available_width(), 50.0);
                let (outer_rect, resp) =
                    ui.allocate_exact_size(bar_size, egui::Sense::click_and_drag());

                let bar_left = outer_rect.left() + 12.0;
                let bar_right = outer_rect.right() - 12.0;
                let bar_w = (bar_right - bar_left).max(1.0);

                let ruler_rect = Rect::from_min_max(
                    Pos2::new(bar_left, outer_rect.top()),
                    Pos2::new(bar_right, outer_rect.top() + 20.0),
                );
                let track_rect = Rect::from_min_max(
                    Pos2::new(bar_left, ruler_rect.bottom() + 2.0),
                    Pos2::new(bar_right, outer_rect.bottom() - 2.0),
                );

                let start_x = bar_left + bar_w * (self.trim_start / dur) as f32;
                let end_x = bar_left + bar_w * (self.trim_end / dur) as f32;

                let pick_target = |pos: Pos2| -> AudioTimelineDrag {
                    if pos.y <= ruler_rect.bottom() {
                        return AudioTimelineDrag::Playhead;
                    }
                    let d_start = (pos.x - start_x).abs();
                    let d_end = (pos.x - end_x).abs();
                    if d_start <= 18.0 || d_end <= 18.0 {
                        if d_start <= d_end {
                            AudioTimelineDrag::TrimStart
                        } else {
                            AudioTimelineDrag::TrimEnd
                        }
                    } else if pos.x > start_x + 18.0 && pos.x < end_x - 18.0 {
                        AudioTimelineDrag::TrimWindow
                    } else {
                        AudioTimelineDrag::Playhead
                    }
                };

                let hovered_target = if self.active_drag != AudioTimelineDrag::None {
                    self.active_drag
                } else if let Some(hover_pos) = resp.hover_pos() {
                    pick_target(hover_pos)
                } else {
                    AudioTimelineDrag::None
                };

                if resp.hover_pos().is_some() || self.active_drag != AudioTimelineDrag::None {
                    let cursor = match hovered_target {
                        AudioTimelineDrag::TrimStart | AudioTimelineDrag::TrimEnd => {
                            CursorIcon::ResizeHorizontal
                        }
                        AudioTimelineDrag::TrimWindow => {
                            if self.active_drag == AudioTimelineDrag::TrimWindow {
                                CursorIcon::Grabbing
                            } else {
                                CursorIcon::Grab
                            }
                        }
                        AudioTimelineDrag::Playhead => CursorIcon::PointingHand,
                        AudioTimelineDrag::None => CursorIcon::Default,
                    };
                    ui.ctx().set_cursor_icon(cursor);
                }

                if resp.drag_started() {
                    let origin = ui
                        .ctx()
                        .input(|i| i.pointer.press_origin())
                        .or_else(|| resp.interact_pointer_pos());
                    if let Some(pos) = origin {
                        self.resume_after_drag = self.is_playing();
                        self.stop_playback();
                        self.active_drag = pick_target(pos);
                        if self.active_drag == AudioTimelineDrag::TrimWindow {
                            let ratio = ((pos.x - bar_left) / bar_w).clamp(0.0, 1.0) as f64;
                            self.drag_window_offset_secs = ratio * dur - self.trim_start;
                        }
                    }
                } else if resp.clicked() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let was_playing = self.is_playing();
                        let ratio = ((pos.x - bar_left) / bar_w).clamp(0.0, 1.0) as f64;
                        self.play_time = ratio * dur;
                        if was_playing {
                            self.start_playback_at(self.play_time);
                        }
                    }
                }

                if resp.dragged() && self.active_drag != AudioTimelineDrag::None {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let ratio = ((pos.x - bar_left) / bar_w).clamp(0.0, 1.0) as f64;
                        let t = ratio * dur;
                        match self.active_drag {
                            AudioTimelineDrag::TrimStart => {
                                self.trim_start = t.min((self.trim_end - 0.1).max(0.0));
                                self.play_time = self.trim_start;
                            }
                            AudioTimelineDrag::TrimEnd => {
                                self.trim_end = t.max((self.trim_start + 0.1).min(dur));
                                self.play_time = self.trim_end;
                            }
                            AudioTimelineDrag::TrimWindow => {
                                let len = (self.trim_end - self.trim_start).max(0.1);
                                let new_start = (t - self.drag_window_offset_secs)
                                    .clamp(0.0, (dur - len).max(0.0));
                                self.trim_start = new_start;
                                self.trim_end = (new_start + len).min(dur);
                                self.play_time = self.trim_start;
                            }
                            AudioTimelineDrag::Playhead => {
                                self.play_time = t;
                            }
                            AudioTimelineDrag::None => {}
                        }
                    }
                }

                if resp.drag_stopped() && self.active_drag != AudioTimelineDrag::None {
                    self.active_drag = AudioTimelineDrag::None;
                    if self.resume_after_drag {
                        self.resume_after_drag = false;
                        self.start_playback_at(self.play_time);
                    }
                }

                // Paint two-zone bar
                let start_x = bar_left + bar_w * (self.trim_start / dur) as f32;
                let end_x = bar_left + bar_w * (self.trim_end / dur) as f32;
                let play_x = bar_left + bar_w * (self.play_time / dur) as f32;
                let p = ui.painter();

                p.line_segment(
                    [
                        Pos2::new(bar_left, ruler_rect.bottom() - 2.0),
                        Pos2::new(bar_right, ruler_rect.bottom() - 2.0),
                    ],
                    Stroke::new(1.0_f32, theme::BORDER_SUBTLE),
                );
                p.rect_filled(track_rect, Rounding::same(7.0), theme::BG_ELEVATED);
                p.rect_stroke(
                    track_rect,
                    Rounding::same(7.0),
                    Stroke::new(1.0_f32, theme::BORDER),
                );

                let active_rect = Rect::from_min_max(
                    Pos2::new(start_x, track_rect.top() + 1.0),
                    Pos2::new(end_x.max(start_x + 6.0), track_rect.bottom() - 1.0),
                );
                p.rect_filled(active_rect, Rounding::same(5.0), theme::ACCENT_SOFT);
                p.rect_stroke(
                    active_rect,
                    Rounding::same(5.0),
                    Stroke::new(1.5_f32, theme::ACCENT),
                );

                let draw_trim_handle = |cx: f32, active: bool| {
                    let h_rect = Rect::from_center_size(
                        Pos2::new(cx, track_rect.center().y),
                        Vec2::new(12.0, track_rect.height() + 2.0),
                    );
                    let fill = if active {
                        theme::ACCENT_HOVER
                    } else {
                        theme::ACCENT
                    };
                    let stroke = if active {
                        Stroke::new(1.5_f32, Color32::WHITE)
                    } else {
                        Stroke::NONE
                    };
                    p.rect_filled(h_rect, Rounding::same(4.0), fill);
                    p.rect_stroke(h_rect, Rounding::same(4.0), stroke);

                    let grip_col = Color32::from_white_alpha(200);
                    let gy0 = h_rect.center().y - 5.0;
                    let gy1 = h_rect.center().y + 5.0;
                    p.line_segment(
                        [Pos2::new(cx - 2.0, gy0), Pos2::new(cx - 2.0, gy1)],
                        Stroke::new(1.0_f32, grip_col),
                    );
                    p.line_segment(
                        [Pos2::new(cx + 2.0, gy0), Pos2::new(cx + 2.0, gy1)],
                        Stroke::new(1.0_f32, grip_col),
                    );
                };

                draw_trim_handle(start_x, hovered_target == AudioTimelineDrag::TrimStart);
                draw_trim_handle(end_x, hovered_target == AudioTimelineDrag::TrimEnd);

                // White playhead needle + top cap
                p.line_segment(
                    [
                        Pos2::new(play_x, ruler_rect.top() + 8.0),
                        Pos2::new(play_x, track_rect.bottom()),
                    ],
                    Stroke::new(2.0_f32, Color32::WHITE),
                );
                let head_pts = vec![
                    Pos2::new(play_x - 6.0, ruler_rect.top() + 2.0),
                    Pos2::new(play_x + 6.0, ruler_rect.top() + 2.0),
                    Pos2::new(play_x + 6.0, ruler_rect.top() + 9.0),
                    Pos2::new(play_x, ruler_rect.top() + 15.0),
                    Pos2::new(play_x - 6.0, ruler_rect.top() + 9.0),
                ];
                p.add(egui::Shape::convex_polygon(
                    head_pts,
                    Color32::WHITE,
                    Stroke::new(1.0_f32, theme::ACCENT),
                ));
            });
    }

    pub fn render_inspector(&mut self, ui: &mut egui::Ui, is_busy: bool) -> Option<AudioExportJob> {
        let mut job = None;
        let dur = self.info.duration_secs.max(0.1);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - 62.0)
            .show(ui, |ui| {
                // Trim card
                theme::section_card(ui, "ОБРЕЗКА ТРЕКА", |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Начало").size(12.0).color(theme::TEXT_SECONDARY));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            theme::editable_time_field(
                                ui,
                                "audio_trim_start",
                                &mut self.trim_start,
                                0.0,
                                (self.trim_end - 0.1).max(0.0),
                            );
                        });
                    });
                    if ui
                        .add(
                            egui::Slider::new(&mut self.trim_start, 0.0..=(dur - 0.1).max(0.0))
                                .show_value(false),
                        )
                        .changed()
                    {
                        if self.trim_start >= self.trim_end - 0.1 {
                            self.trim_end = (self.trim_start + 0.1).min(dur);
                        }
                    }

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Конец").size(12.0).color(theme::TEXT_SECONDARY));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            theme::editable_time_field(
                                ui,
                                "audio_trim_end",
                                &mut self.trim_end,
                                (self.trim_start + 0.1).min(dur),
                                dur,
                            );
                        });
                    });
                    if ui
                        .add(egui::Slider::new(&mut self.trim_end, 0.1..=dur).show_value(false))
                        .changed()
                    {
                        if self.trim_end <= self.trim_start + 0.1 {
                            self.trim_start = (self.trim_end - 0.1).max(0.0);
                        }
                    }
                });

                ui.add_space(10.0);

                // Volume card
                theme::section_card(ui, "ГРОМКОСТЬ", |ui| {
                    let db_text = if self.volume_pct <= 0.5 {
                        "−∞ дБ".to_string()
                    } else {
                        let db = 20.0 * (self.volume_pct / 100.0).log10();
                        format!("{:+.1} дБ", db)
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{:.0}%", self.volume_pct))
                                .size(12.5)
                                .color(theme::TEXT_PRIMARY),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(db_text).size(11.5).color(theme::TEXT_MUTED));
                        });
                    });
                    if theme::volume_slider_with_steps(ui, &mut self.volume_pct) {
                        self.sync_live_audio();
                    }
                });

                ui.add_space(10.0);

                // Fade In / Out card
                let max_fade = ((self.trim_end - self.trim_start) * 0.5).clamp(0.5, 15.0);
                self.fade_in = self.fade_in.min(max_fade);
                self.fade_out = self.fade_out.min(max_fade);

                theme::section_card(ui, "ПЛАВНЫЕ ПЕРЕХОДЫ", |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Появление (Fade In)").size(12.0).color(theme::TEXT_SECONDARY));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("{:.1} с", self.fade_in))
                                    .size(12.0)
                                    .color(theme::TEXT_PRIMARY),
                            );
                        });
                    });
                    ui.add(egui::Slider::new(&mut self.fade_in, 0.0..=max_fade).show_value(false));

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Затухание (Fade Out)").size(12.0).color(theme::TEXT_SECONDARY));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("{:.1} с", self.fade_out))
                                    .size(12.0)
                                    .color(theme::TEXT_PRIMARY),
                            );
                        });
                    });
                    ui.add(egui::Slider::new(&mut self.fade_out, 0.0..=max_fade).show_value(false));
                });

                ui.add_space(10.0);

                // Format card
                theme::section_card(ui, "ФОРМАТ СОХРАНЕНИЯ", |ui| {
                    ui.horizontal(|ui| {
                        for fmt in [
                            AudioFormat::Mp3,
                            AudioFormat::Wav,
                            AudioFormat::Ogg,
                            AudioFormat::M4a,
                        ] {
                            if theme::pill_choice(ui, self.format == fmt, fmt.label()).clicked() {
                                self.format = fmt;
                            }
                        }
                    });
                });

                ui.add_space(8.0);
                if theme::subtle_button(ui, "Сбросить настройки").clicked() {
                    self.reset_edits();
                }
                ui.add_space(6.0);
            });

        ui.add_space(8.0);
        let btn_text = if is_busy {
            "Обработка аудио..."
        } else {
            "Экспортировать аудио"
        };

        if let Some(picked) = self.save_dialog.as_ref().and_then(|d| d.poll()) {
            self.save_dialog = None;
            if let (Some(input), Some(output)) = (self.path.clone(), picked) {
                job = Some(AudioExportJob {
                    input,
                    output,
                    trim_start: self.trim_start,
                    trim_end: self.trim_end,
                    volume_pct: self.volume_pct,
                    fade_in: self.fade_in,
                    fade_out: self.fade_out,
                    format: self.format,
                });
            }
        }

        ui.add_enabled_ui(!is_busy && self.save_dialog.is_none(), |ui| {
            if theme::accent_button(ui, btn_text).clicked() {
                self.request_save(ui.ctx());
            }
        });

        job
    }
}

pub fn run_audio_export(job: &AudioExportJob) -> Result<PathBuf, String> {
    let mut cmd = media::hidden_command("ffmpeg");
    cmd.arg("-y");

    if job.trim_start > 0.01 {
        cmd.args(["-ss", &format!("{:.3}", job.trim_start)]);
    }

    cmd.arg("-i").arg(&job.input);

    let duration = (job.trim_end - job.trim_start).max(0.1);
    cmd.args(["-t", &format!("{:.3}", duration)]);
    cmd.arg("-vn");

    let mut af_parts: Vec<String> = Vec::new();
    if (job.volume_pct - 100.0).abs() > 0.5 {
        af_parts.push(format!("volume={:.3}", (job.volume_pct / 100.0).max(0.0)));
    }
    if job.fade_in > 0.05 {
        af_parts.push(format!("afade=t=in:st=0:d={:.2}", job.fade_in));
    }
    if job.fade_out > 0.05 {
        let st = (duration - job.fade_out).max(0.0);
        af_parts.push(format!("afade=t=out:st={:.2}:d={:.2}", st, job.fade_out));
    }

    if !af_parts.is_empty() {
        cmd.args(["-af", &af_parts.join(",")]);
    }

    match job.format {
        AudioFormat::Mp3 => {
            cmd.args(["-c:a", "libmp3lame", "-q:a", "2"]);
        }
        AudioFormat::Wav => {
            cmd.args(["-c:a", "pcm_s16le"]);
        }
        AudioFormat::Ogg => {
            cmd.args(["-c:a", "libvorbis", "-q:a", "5"]);
        }
        AudioFormat::M4a => {
            cmd.args(["-c:a", "aac", "-b:a", "192k"]);
        }
    }

    cmd.arg(&job.output);
    execute_ffmpeg(cmd, &job.output)
}

fn execute_ffmpeg(mut cmd: std::process::Command, out: &Path) -> Result<PathBuf, String> {
    let output = cmd
        .output()
        .map_err(|e| format!("Не удалось запустить ffmpeg: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last_line = stderr.lines().last().unwrap_or("Неизвестная ошибка ffmpeg");
        return Err(format!("Ошибка экспорта: {}", last_line));
    }
    Ok(out.to_path_buf())
}
