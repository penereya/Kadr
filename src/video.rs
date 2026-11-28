use crate::media::{self, MediaInfo};
use crate::theme;
use eframe::egui::{
    self, Color32, CursorIcon, Pos2, Rect, Rounding, Stroke, TextureHandle, TextureOptions, Vec2,
};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VideoFormat {
    Mp4,
    Mov,
    Webm,
    Gif,
}

impl VideoFormat {
    pub fn ext(&self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "mp4",
            VideoFormat::Mov => "mov",
            VideoFormat::Webm => "webm",
            VideoFormat::Gif => "gif",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "MP4",
            VideoFormat::Mov => "MOV",
            VideoFormat::Webm => "WEBM",
            VideoFormat::Gif => "GIF",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VideoQuality {
    Original,
    High,
    Balanced,
    Compact,
}

impl VideoQuality {
    pub fn label(&self) -> &'static str {
        match self {
            VideoQuality::Original => "Оригинал",
            VideoQuality::High => "Высокое",
            VideoQuality::Balanced => "Баланс",
            VideoQuality::Compact => "Компакт",
        }
    }

    pub fn crf(&self) -> &'static str {
        match self {
            VideoQuality::Original => "18",
            VideoQuality::High => "21",
            VideoQuality::Balanced => "26",
            VideoQuality::Compact => "32",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VideoAudioMode {
    Keep,
    Mute,
    ExtractMp3,
}

impl VideoAudioMode {
    pub fn label(&self) -> &'static str {
        match self {
            VideoAudioMode::Keep => "Оставить",
            VideoAudioMode::Mute => "Убрать",
            VideoAudioMode::ExtractMp3 => "Только MP3",
        }
    }
}

pub enum VideoCanvasAction {
    None,
    Deleted(String),
    DeleteError(String),
}

#[derive(Clone)]
pub struct VideoExportJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub trim_start: f64,
    pub trim_end: f64,
    pub rotation_deg: u32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub quality: VideoQuality,
    pub audio_mode: VideoAudioMode,
    pub volume_pct: f32,
    pub format: VideoFormat,
    pub delete_original_after_export: bool,
    pub source_video_codec: String,
    pub source_audio_codec: String,
    pub has_audio: bool,
    pub source_ext: String,
    pub target_video_kbps: Option<u32>,
}

pub struct VideoExportResult {
    pub saved_path: PathBuf,
    pub deleted_original: Option<PathBuf>,
    pub warning: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimelineDrag {
    None,
    Playhead,
    TrimStart,
    TrimEnd,
    TrimWindow,
}

struct PlaybackFrame {
    image: egui::ColorImage,
    time_secs: f64,
}

struct VideoPlayer {
    stop_flag: Arc<AtomicBool>,
    volume_bits: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    frame_rx: Receiver<PlaybackFrame>,
    video_child: Arc<Mutex<Option<Child>>>,
    audio_child: Arc<Mutex<Option<Child>>>,
    finished: Arc<AtomicBool>,
    end_time: f64,
}

impl VideoPlayer {
    #[allow(clippy::too_many_arguments)]
    fn start(
        path: &Path,
        start_time: f64,
        end_time: f64,
        width: u32,
        height: u32,
        src_fps: f64,
        has_audio: bool,
        play_audio: bool,
        volume_pct: f32,
        ctx: egui::Context,
    ) -> Result<Self, String> {
        let (src_w, src_h) = (width.max(2), height.max(2));

        let max_w = 1920u32;
        let (out_w, out_h) = if src_w > max_w {
            let h = ((src_h as f64 * max_w as f64 / src_w as f64).round() as u32) & !1;
            (max_w, h.max(2))
        } else {
            ((src_w & !1).max(2), (src_h & !1).max(2))
        };

        let fps = if src_fps > 1.0 {
            src_fps.clamp(15.0, 30.0)
        } else {
            30.0
        };
        let vf = if out_w == src_w && out_h == src_h {
            format!("fps={}", fps.round() as u32)
        } else {
            format!("scale={}:{}:flags=bilinear,fps={}", out_w, out_h, fps.round() as u32)
        };

        let duration = (end_time - start_time).max(0.1);

        let mut vcmd = media::hidden_command("ffmpeg");
        vcmd.args([
            "-v",
            "error",
            "-ss",
            &format!("{:.3}", start_time.max(0.0)),
            "-i",
        ])
        .arg(path)
        .args([
            "-t",
            &format!("{:.3}", duration),
            "-an",
            "-vf",
            &vf,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

        let mut vchild = vcmd
            .spawn()
            .map_err(|e| format!("Не удалось запустить воспроизведение: {}", e))?;

        let mut stdout = vchild
            .stdout
            .take()
            .ok_or_else(|| "Не удалось открыть поток видео".to_string())?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let volume_bits = Arc::new(AtomicU32::new(volume_pct.to_bits()));
        let muted = Arc::new(AtomicBool::new(!play_audio));

        // Synchronized audio stream via Windows waveOut with lock-free live volume control
        let achild_opt = if has_audio {
            media::spawn_live_audio_stream(
                path.to_path_buf(),
                start_time,
                duration,
                Arc::clone(&volume_bits),
                Arc::clone(&muted),
                Arc::clone(&stop_flag),
            )
        } else {
            None
        };

        let finished = Arc::new(AtomicBool::new(false));
        let video_child = Arc::new(Mutex::new(Some(vchild)));
        let audio_child = achild_opt.unwrap_or_else(|| Arc::new(Mutex::new(None)));

        let (tx, rx) = mpsc::sync_channel::<PlaybackFrame>(2);

        let stop_clone = Arc::clone(&stop_flag);
        let finished_clone = Arc::clone(&finished);
        let vchild_clone = Arc::clone(&video_child);
        let achild_clone = Arc::clone(&audio_child);

        std::thread::spawn(move || {
            let frame_bytes = (out_w as usize) * (out_h as usize) * 4;
            let mut buf = vec![0u8; frame_bytes];
            let started_at = Instant::now();
            let mut frame_idx: u64 = 0;

            loop {
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }

                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }

                let target_elapsed = frame_idx as f64 / fps;
                frame_idx += 1;

                // Pace frames to real time
                loop {
                    if stop_clone.load(Ordering::Relaxed) {
                        break;
                    }
                    let elapsed = started_at.elapsed().as_secs_f64();
                    if elapsed >= target_elapsed {
                        break;
                    }
                    let remain = (target_elapsed - elapsed).min(0.015);
                    std::thread::sleep(Duration::from_secs_f64(remain));
                }

                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }

                // Skip pushing if we fell behind significantly so audio/video stay in sync
                let elapsed = started_at.elapsed().as_secs_f64();
                if elapsed > target_elapsed + 0.12 {
                    continue;
                }

                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [out_w as usize, out_h as usize],
                    &buf,
                );
                let time_secs = (start_time + target_elapsed).min(end_time);

                // Non-blocking send so thread never stalls if UI is busy
                let _ = tx.try_send(PlaybackFrame { image, time_secs });
                ctx.request_repaint();
            }

            finished_clone.store(true, Ordering::Relaxed);
            if let Ok(mut guard) = vchild_clone.lock() {
                if let Some(mut c) = guard.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
            }
            if let Ok(mut guard) = achild_clone.lock() {
                if let Some(mut c) = guard.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
            }
            ctx.request_repaint();
        });

        Ok(Self {
            stop_flag,
            volume_bits,
            muted,
            frame_rx: rx,
            video_child,
            audio_child,
            finished,
            end_time,
        })
    }

    fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.video_child.lock() {
            if let Some(mut c) = guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        if let Ok(mut guard) = self.audio_child.lock() {
            if let Some(mut c) = guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct VideoEditor {
    pub path: Option<PathBuf>,
    pub info: MediaInfo,
    preview_texture: Option<TextureHandle>,
    pub preview_time: f64,
    pub preview_error: Option<String>,

    // Async single-frame preview loading (so dragging never lags the UI)
    pending_frame_rx: Option<Receiver<Result<egui::ColorImage, String>>>,
    last_scrub_fetch: Instant,
    needs_scrub_fetch: bool,

    // Live playback
    player: Option<VideoPlayer>,
    active_drag: TimelineDrag,
    drag_window_offset_secs: f64,
    resume_after_drag: bool,
    confirm_delete_open: bool,

    pub trim_start: f64,
    pub trim_end: f64,
    pub rotation_deg: u32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub quality: VideoQuality,
    pub audio_mode: VideoAudioMode,
    pub volume_pct: f32,
    pub format: VideoFormat,
    pub delete_original_after_export: bool,
    save_dialog: Option<media::PendingFileDialog>,
    history: media::UndoHistory<VideoSnapshot>,
    frame_snapshot: Option<VideoSnapshot>,
}

#[derive(Clone, PartialEq)]
pub struct VideoSnapshot {
    trim_start: f64,
    trim_end: f64,
    rotation_deg: u32,
    flip_h: bool,
    flip_v: bool,
    quality: VideoQuality,
    audio_mode: VideoAudioMode,
    volume_pct: f32,
    format: VideoFormat,
    delete_original_after_export: bool,
}

impl Default for VideoEditor {
    fn default() -> Self {
        Self {
            path: None,
            info: MediaInfo::default(),
            preview_texture: None,
            preview_time: 0.0,
            preview_error: None,
            pending_frame_rx: None,
            last_scrub_fetch: Instant::now(),
            needs_scrub_fetch: false,
            player: None,
            active_drag: TimelineDrag::None,
            drag_window_offset_secs: 0.0,
            resume_after_drag: false,
            confirm_delete_open: false,
            trim_start: 0.0,
            trim_end: 1.0,
            rotation_deg: 0,
            flip_h: false,
            flip_v: false,
            quality: VideoQuality::Balanced,
            audio_mode: VideoAudioMode::Keep,
            volume_pct: 15.0,
            format: VideoFormat::Mp4,
            delete_original_after_export: false,
            save_dialog: None,
            history: media::UndoHistory::default(),
            frame_snapshot: None,
        }
    }
}

impl VideoEditor {
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

    pub fn clear(&mut self) {
        self.stop_playback();
        self.path = None;
        self.info = MediaInfo::default();
        self.preview_texture = None;
        self.pending_frame_rx = None;
        self.preview_error = None;
        self.confirm_delete_open = false;
        self.active_drag = TimelineDrag::None;
        self.resume_after_drag = false;
        self.needs_scrub_fetch = false;
        self.save_dialog = None;
        self.history.clear();
        self.frame_snapshot = None;
    }

    pub fn snapshot(&self) -> VideoSnapshot {
        VideoSnapshot {
            trim_start: self.trim_start,
            trim_end: self.trim_end,
            rotation_deg: self.rotation_deg,
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            quality: self.quality,
            audio_mode: self.audio_mode,
            volume_pct: self.volume_pct,
            format: self.format,
            delete_original_after_export: self.delete_original_after_export,
        }
    }

    pub fn apply_snapshot(&mut self, s: VideoSnapshot) {
        self.trim_start = s.trim_start;
        self.trim_end = s.trim_end;
        self.rotation_deg = s.rotation_deg;
        self.flip_h = s.flip_h;
        self.flip_v = s.flip_v;
        self.quality = s.quality;
        self.audio_mode = s.audio_mode;
        self.volume_pct = s.volume_pct;
        self.format = s.format;
        self.delete_original_after_export = s.delete_original_after_export;
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

    pub fn seek_by(&mut self, delta_secs: f64, ctx: &egui::Context) {
        let dur = self.info.duration_secs.max(0.1);
        let target = (self.preview_time + delta_secs).clamp(0.0, dur);
        self.preview_time = target;
        if self.is_playing() {
            self.start_playback_at(target, ctx);
        } else {
            self.request_frame_async(ctx);
        }
    }

    pub fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.confirm_delete_open || ctx.wants_keyboard_input() {
            return;
        }
        let (toggle, seek_delta, go_home, shift_home, go_end, set_in, set_out, mute, vol_delta, rot_l, rot_r, flip_h, flip_v, del) =
            ctx.input(|i| {
                let no_ctrl = !i.modifiers.ctrl && !i.modifiers.command && !i.modifiers.alt;
                let step = if i.modifiers.shift { 0.5 } else { 5.0 };
                let mut seek = 0.0;
                if no_ctrl && i.key_pressed(egui::Key::ArrowLeft) {
                    seek -= step;
                }
                if no_ctrl && i.key_pressed(egui::Key::ArrowRight) {
                    seek += step;
                }
                let mut vol = 0.0_f32;
                if no_ctrl && i.key_pressed(egui::Key::ArrowUp) {
                    vol += 5.0;
                }
                if no_ctrl && i.key_pressed(egui::Key::ArrowDown) {
                    vol -= 5.0;
                }
                (
                    no_ctrl && i.key_pressed(egui::Key::Space),
                    seek,
                    no_ctrl && i.key_pressed(egui::Key::Home),
                    i.modifiers.shift,
                    no_ctrl && i.key_pressed(egui::Key::End),
                    no_ctrl && i.key_pressed(egui::Key::I),
                    no_ctrl && i.key_pressed(egui::Key::O),
                    no_ctrl && i.key_pressed(egui::Key::M),
                    vol,
                    no_ctrl && i.key_pressed(egui::Key::L),
                    no_ctrl && i.key_pressed(egui::Key::R),
                    no_ctrl && i.key_pressed(egui::Key::H),
                    no_ctrl && i.key_pressed(egui::Key::V),
                    no_ctrl && i.key_pressed(egui::Key::Delete),
                )
            });

        if toggle {
            self.toggle_playback(ctx);
        }
        if seek_delta != 0.0 {
            self.seek_by(seek_delta, ctx);
        }
        if go_home {
            let target = if shift_home { 0.0 } else { self.trim_start };
            self.preview_time = target;
            if self.is_playing() {
                self.start_playback_at(target, ctx);
            } else {
                self.request_frame_async(ctx);
            }
        }
        if go_end {
            let target = self.trim_end;
            self.preview_time = target;
            if self.is_playing() {
                self.start_playback_at(target, ctx);
            } else {
                self.request_frame_async(ctx);
            }
        }
        if set_in {
            self.trim_start = self.preview_time.clamp(0.0, (self.trim_end - 0.1).max(0.0));
        }
        if set_out {
            let dur = self.info.duration_secs.max(0.1);
            self.trim_end = self.preview_time.clamp((self.trim_start + 0.1).min(dur), dur);
        }
        if mute {
            self.audio_mode = if self.audio_mode == VideoAudioMode::Mute {
                VideoAudioMode::Keep
            } else {
                VideoAudioMode::Mute
            };
            self.sync_live_audio();
        }
        if vol_delta != 0.0 {
            self.volume_pct = (self.volume_pct + vol_delta).clamp(0.0, 300.0);
            self.sync_live_audio();
        }
        if rot_l {
            self.rotation_deg = (self.rotation_deg + 270) % 360;
        }
        if rot_r {
            self.rotation_deg = (self.rotation_deg + 90) % 360;
        }
        if flip_h {
            self.flip_h = !self.flip_h;
        }
        if flip_v {
            self.flip_v = !self.flip_v;
        }
        if del {
            self.stop_playback();
            self.confirm_delete_open = true;
        }
    }

    pub fn request_save(&mut self, ctx: &egui::Context) {
        if !self.has_file() || self.save_dialog.is_some() {
            return;
        }
        self.stop_playback();
        let ext = if self.audio_mode == VideoAudioMode::ExtractMp3 {
            "mp3"
        } else {
            self.format.ext()
        };
        let filter_name = if self.audio_mode == VideoAudioMode::ExtractMp3 {
            "MP3"
        } else {
            self.format.label()
        };
        let default_name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(|s| format!("{}_edited.{}", s, ext))
            .unwrap_or_else(|| format!("video_edited.{}", ext));

        self.save_dialog = Some(media::PendingFileDialog::spawn(ctx, move || {
            rfd::FileDialog::new()
                .set_file_name(&default_name)
                .add_filter(filter_name, &[ext])
                .save_file()
        }));
    }

    fn sync_live_audio(&self) {
        if let Some(player) = &self.player {
            player
                .volume_bits
                .store(self.volume_pct.to_bits(), Ordering::Relaxed);
            player
                .muted
                .store(self.audio_mode == VideoAudioMode::Mute, Ordering::Relaxed);
        }
    }

    fn start_playback_at(&mut self, start: f64, ctx: &egui::Context) {
        self.stop_playback();
        let Some(path) = self.path.clone() else { return };
        let dur = self.info.duration_secs.max(0.1);
        let clamped_start = start.clamp(0.0, (dur - 0.1).max(0.0));
        let end = if clamped_start < self.trim_end - 0.05 {
            self.trim_end.min(dur)
        } else {
            dur
        };
        let play_audio = self.info.has_audio && self.audio_mode != VideoAudioMode::Mute;

        match VideoPlayer::start(
            &path,
            clamped_start,
            end.max(clamped_start + 0.1),
            self.info.width,
            self.info.height,
            self.info.fps,
            self.info.has_audio,
            play_audio,
            self.volume_pct,
            ctx.clone(),
        ) {
            Ok(p) => {
                self.preview_time = clamped_start;
                self.preview_error = None;
                self.player = Some(p);
            }
            Err(e) => {
                self.preview_error = Some(e);
            }
        }
    }

    pub fn toggle_playback(&mut self, ctx: &egui::Context) {
        if self.is_playing() {
            self.stop_playback();
            self.request_frame_async(ctx);
            return;
        }

        let dur = self.info.duration_secs.max(0.1);
        let at_trim_end = (self.preview_time - self.trim_end).abs() < 0.08;
        let at_video_end = self.preview_time >= dur - 0.15;

        // Only jump back to trim_start if playhead finished at trim_end or end of video;
        // otherwise respect any manual playhead placement (even outside trim!).
        let start = if at_trim_end || at_video_end {
            self.trim_start
        } else {
            self.preview_time
        };
        self.start_playback_at(start, ctx);
    }

    pub fn estimate_output_bytes(&self) -> u64 {
        let clip_len = (self.trim_end - self.trim_start).max(0.1);

        if self.audio_mode == VideoAudioMode::ExtractMp3 {
            return ((190_000.0 / 8.0) * clip_len) as u64;
        }

        if self.format == VideoFormat::Gif {
            let w = self.info.width.clamp(320, 640) as f64;
            let gif_kbps = 2400.0 * (w / 640.0).powf(1.4);
            return ((gif_kbps * 1000.0 / 8.0) * clip_len) as u64;
        }

        if self.quality == VideoQuality::Original {
            let ratio = (clip_len / self.info.duration_secs.max(0.1)).clamp(0.0, 1.0);
            let mut est = self.info.file_size as f64 * ratio;
            if self.format == VideoFormat::Webm {
                est *= 0.85;
            }
            if self.audio_mode == VideoAudioMode::Mute && self.info.has_audio {
                est = (est - (192_000.0 / 8.0) * clip_len).max(0.0);
            }
            return est as u64;
        }

        let video_kbps = compute_target_video_kbps(&self.info, self.quality, self.format)
            .unwrap_or(2600) as f64;
        let audio_kbps = if !self.info.has_audio || self.audio_mode == VideoAudioMode::Mute {
            0.0
        } else if self.format == VideoFormat::Webm {
            128.0
        } else {
            192.0
        };

        (((video_kbps + audio_kbps) * 1000.0 / 8.0) * clip_len * 1.01) as u64
    }

    pub fn load(&mut self, path: PathBuf, ctx: &egui::Context) -> Result<(), String> {
        self.stop_playback();
        let info = media::probe_media(&path)?;
        self.info = info.clone();
        self.path = Some(path);
        self.trim_start = 0.0;
        self.trim_end = info.duration_secs.max(0.1);
        self.preview_time = (info.duration_secs * 0.05).min(1.0);
        self.rotation_deg = 0;
        self.flip_h = false;
        self.flip_v = false;
        self.quality = VideoQuality::Balanced;
        self.audio_mode = VideoAudioMode::Keep;
        self.volume_pct = 15.0;
        self.format = VideoFormat::Mp4;
        self.confirm_delete_open = false;
        self.active_drag = TimelineDrag::None;
        self.save_dialog = None;
        self.history.clear();
        self.frame_snapshot = None;
        self.request_frame_async(ctx);
        Ok(())
    }

    pub fn request_frame_async(&mut self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else { return };
        let time = self.preview_time;
        let ctx_clone = ctx.clone();

        let (tx, rx) = mpsc::channel();
        self.pending_frame_rx = Some(rx);
        self.last_scrub_fetch = Instant::now();
        self.needs_scrub_fetch = false;

        std::thread::spawn(move || {
            let res = media::extract_video_frame(&path, time, 0, false, false);
            let _ = tx.send(res);
            ctx_clone.request_repaint();
        });
    }

    fn poll_frames(&mut self, ctx: &egui::Context) {
        // 1. Poll live playback frames
        let mut stop_at_end = false;
        let mut ended_naturally = false;
        if let Some(player) = &self.player {
            let mut latest: Option<PlaybackFrame> = None;
            while let Ok(frame) = player.frame_rx.try_recv() {
                latest = Some(frame);
            }
            if let Some(frame) = latest {
                self.preview_time = frame.time_secs.min(player.end_time);
                if let Some(tex) = &mut self.preview_texture {
                    tex.set(frame.image, TextureOptions::LINEAR);
                } else {
                    self.preview_texture = Some(ctx.load_texture(
                        "video_preview",
                        frame.image,
                        TextureOptions::LINEAR,
                    ));
                }
                if frame.time_secs >= player.end_time - 0.02 {
                    stop_at_end = true;
                }
            }
            if player.finished.load(Ordering::Relaxed) {
                self.player = None;
                ended_naturally = true;
            }
        }
        if stop_at_end {
            self.stop_playback();
        }
        if stop_at_end || ended_naturally {
            self.request_frame_async(ctx);
        }

        // 2. Poll async single-frame scrub result
        if let Some(rx) = &self.pending_frame_rx {
            if let Ok(res) = rx.try_recv() {
                self.pending_frame_rx = None;
                match res {
                    Ok(img) => {
                        if let Some(tex) = &mut self.preview_texture {
                            tex.set(img, TextureOptions::LINEAR);
                        } else {
                            self.preview_texture = Some(ctx.load_texture(
                                "video_preview",
                                img,
                                TextureOptions::LINEAR,
                            ));
                        }
                        self.preview_error = None;
                    }
                    Err(e) => {
                        self.preview_error = Some(e);
                    }
                }
            }
        }

        // 3. Throttled frame fetch while dragging playhead/trim
        if self.needs_scrub_fetch
            && self.pending_frame_rx.is_none()
            && self.last_scrub_fetch.elapsed() >= Duration::from_millis(120)
        {
            self.request_frame_async(ctx);
        }
    }

    pub fn reset_edits(&mut self, ctx: &egui::Context) {
        self.stop_playback();
        self.trim_start = 0.0;
        self.trim_end = self.info.duration_secs.max(0.1);
        self.rotation_deg = 0;
        self.flip_h = false;
        self.flip_v = false;
        self.quality = VideoQuality::Balanced;
        self.audio_mode = VideoAudioMode::Keep;
        self.volume_pct = 15.0;
        self.format = VideoFormat::Mp4;
        self.delete_original_after_export = false;
        self.preview_time = (self.info.duration_secs * 0.05).min(1.0);
        self.request_frame_async(ctx);
    }

    pub fn render_canvas(&mut self, ui: &mut egui::Ui) -> VideoCanvasAction {
        self.poll_frames(ui.ctx());
        let mut action = VideoCanvasAction::None;

        if self.confirm_delete_open && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            self.confirm_delete_open = false;
        }

        // Top metadata bar with filename, Delete/Close buttons, and file metadata
        let compact_bar = ui.available_width() < 420.0;
        let mut closed = false;
        ui.horizontal_wrapped(|ui| {
            if let Some(p) = &self.path {
                let full_name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Видео");
                let max_chars = if compact_bar { 18 } else { 44 };
                let display_name = truncate_display(full_name, max_chars);
                let lbl = ui.label(
                    egui::RichText::new(display_name)
                        .size(13.5)
                        .color(theme::TEXT_PRIMARY)
                        .strong(),
                );
                if full_name.chars().count() > max_chars {
                    lbl.on_hover_text(full_name);
                }

                if theme::danger_button(ui, "Удалить")
                    .on_hover_text("Удалить это видео в корзину")
                    .clicked()
                {
                    self.stop_playback();
                    self.confirm_delete_open = true;
                }
                if theme::subtle_button(ui, "Закрыть")
                    .on_hover_text("Закрыть файл без удаления")
                    .clicked()
                {
                    self.clear();
                    closed = true;
                }
            }

            if !closed {
                let remaining = ui.max_rect().right() - ui.cursor().left();
                if remaining >= 220.0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme::metadata_pill(ui, &theme::format_bytes(self.info.file_size));
                        if self.info.width > 0 && self.info.height > 0 {
                            theme::metadata_pill(
                                ui,
                                &format!("{} × {}", self.info.width, self.info.height),
                            );
                        }
                        theme::metadata_pill(ui, &theme::format_time(self.info.duration_secs));
                    });
                }
            }
        });

        if closed || !self.has_file() {
            return action;
        }

        ui.add_space(if compact_bar { 6.0 } else { 10.0 });

        let timeline_height = if ui.available_height() < 320.0 { 108.0 } else { 122.0 };
        let preview_height = (ui.available_height() - timeline_height - 12.0).max(80.0);

        // Video preview frame
        egui::Frame::none()
            .fill(Color32::from_rgb(14, 14, 15))
            .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
            .rounding(Rounding::same(14.0))
            .show(ui, |ui| {
                let size = Vec2::new(ui.available_width(), preview_height);
                let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                let padded = rect.shrink(14.0);

                // Clicking video toggles Play / Pause
                if resp.clicked() && !self.confirm_delete_open {
                    self.toggle_playback(ui.ctx());
                }

                if let Some(tex) = &self.preview_texture {
                    let tex_size = tex.size_vec2();
                    if tex_size.x > 0.0 && tex_size.y > 0.0 {
                        let disp_size = if self.rotation_deg == 90 || self.rotation_deg == 270 {
                            Vec2::new(tex_size.y, tex_size.x)
                        } else {
                            tex_size
                        };
                        let scale = (padded.width() / disp_size.x).min(padded.height() / disp_size.y);
                        let draw_size = disp_size * scale;
                        let img_rect = Rect::from_center_size(padded.center(), draw_size);
                        paint_transformed_texture(
                            ui.painter(),
                            tex.id(),
                            img_rect,
                            self.rotation_deg,
                            self.flip_h,
                            self.flip_v,
                        );
                    }
                } else {
                    let msg = self
                        .preview_error
                        .as_deref()
                        .unwrap_or("Загрузка кадра...");
                    ui.painter().text(
                        padded.center(),
                        egui::Align2::CENTER_CENTER,
                        msg,
                        egui::FontId::proportional(14.0),
                        theme::TEXT_MUTED,
                    );
                }
            });

        ui.add_space(10.0);

        // Two-zone interactive timeline + playback bar
        egui::Frame::none()
            .fill(theme::BG_SURFACE)
            .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
            .rounding(Rounding::same(12.0))
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                let dur = self.info.duration_secs.max(0.1);
                let clip_len = (self.trim_end - self.trim_start).max(0.0);

                // Top row of timeline: Play/Pause, compact volume control, fragment info, current time
                ui.horizontal(|ui| {
                    let play_label = if self.is_playing() {
                        "⏸  Пауза"
                    } else {
                        "▶  Смотреть"
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
                        self.toggle_playback(ui.ctx());
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
                        let was_playing = self.is_playing();
                        self.preview_time = target;
                        if was_playing {
                            self.start_playback_at(target, ui.ctx());
                        } else {
                            self.request_frame_async(ui.ctx());
                        }
                    }

                    let narrow_bar = ui.max_rect().width() < 640.0;
                    if self.audio_mode != VideoAudioMode::Mute {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Звук")
                                .size(12.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                        let vol_resp = ui
                            .scope(|ui| {
                                ui.spacing_mut().slider_width = if narrow_bar { 52.0 } else { 78.0 };
                                ui.add(
                                    egui::Slider::new(&mut self.volume_pct, 0.0..=300.0)
                                        .fixed_decimals(0)
                                        .show_value(false),
                                )
                            })
                            .inner;
                        if vol_resp.changed() {
                            self.sync_live_audio();
                        }
                        ui.label(
                            egui::RichText::new(format!("{:.0}%", self.volume_pct))
                                .size(11.5)
                                .color(theme::TEXT_PRIMARY),
                        );
                    }

                    let rem_after_vol = ui.max_rect().right() - ui.cursor().left();
                    if !narrow_bar && rem_after_vol >= 320.0 {
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
                    } else if rem_after_vol >= 185.0 {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("Итого: {}", theme::format_time(clip_len)))
                                .size(11.5)
                                .color(theme::TEXT_SECONDARY),
                        );
                    }

                    if ui.max_rect().right() - ui.cursor().left() >= 95.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} / {}",
                                    theme::format_time(self.preview_time),
                                    theme::format_time(dur)
                                ))
                                .size(12.0)
                                .color(theme::TEXT_PRIMARY),
                            );
                        });
                    }
                });

                ui.add_space(8.0);

                // Two-zone draggable timeline bar (top ruler for white playhead, bottom track for blue trim handles)
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

                let pick_target = |pos: Pos2| -> TimelineDrag {
                    if pos.y <= ruler_rect.bottom() {
                        return TimelineDrag::Playhead;
                    }
                    let d_start = (pos.x - start_x).abs();
                    let d_end = (pos.x - end_x).abs();
                    if d_start <= 18.0 || d_end <= 18.0 {
                        if d_start <= d_end {
                            TimelineDrag::TrimStart
                        } else {
                            TimelineDrag::TrimEnd
                        }
                    } else if pos.x > start_x + 18.0 && pos.x < end_x - 18.0 {
                        TimelineDrag::TrimWindow
                    } else {
                        TimelineDrag::Playhead
                    }
                };

                let hovered_target = if self.active_drag != TimelineDrag::None {
                    self.active_drag
                } else if let Some(hover_pos) = resp.hover_pos() {
                    pick_target(hover_pos)
                } else {
                    TimelineDrag::None
                };

                if resp.hover_pos().is_some() || self.active_drag != TimelineDrag::None {
                    let cursor = match hovered_target {
                        TimelineDrag::TrimStart | TimelineDrag::TrimEnd => {
                            CursorIcon::ResizeHorizontal
                        }
                        TimelineDrag::TrimWindow => {
                            if self.active_drag == TimelineDrag::TrimWindow {
                                CursorIcon::Grabbing
                            } else {
                                CursorIcon::Grab
                            }
                        }
                        TimelineDrag::Playhead => CursorIcon::PointingHand,
                        TimelineDrag::None => CursorIcon::Default,
                    };
                    ui.ctx().set_cursor_icon(cursor);
                }

                // Start drag from exact press origin so fast drags never miss the handle
                if resp.drag_started() {
                    let origin = ui
                        .ctx()
                        .input(|i| i.pointer.press_origin())
                        .or_else(|| resp.interact_pointer_pos());
                    if let Some(pos) = origin {
                        self.resume_after_drag = self.is_playing();
                        self.stop_playback();
                        self.active_drag = pick_target(pos);
                        if self.active_drag == TimelineDrag::TrimWindow {
                            let ratio = ((pos.x - bar_left) / bar_w).clamp(0.0, 1.0) as f64;
                            self.drag_window_offset_secs = ratio * dur - self.trim_start;
                        }
                    }
                } else if resp.clicked() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let was_playing = self.is_playing();
                        let ratio = ((pos.x - bar_left) / bar_w).clamp(0.0, 1.0) as f64;
                        self.preview_time = ratio * dur;
                        if was_playing {
                            self.start_playback_at(self.preview_time, ui.ctx());
                        } else {
                            self.request_frame_async(ui.ctx());
                        }
                    }
                }

                // Smooth drag update
                if resp.dragged() && self.active_drag != TimelineDrag::None {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let ratio = ((pos.x - bar_left) / bar_w).clamp(0.0, 1.0) as f64;
                        let t = ratio * dur;
                        match self.active_drag {
                            TimelineDrag::TrimStart => {
                                self.trim_start = t.min((self.trim_end - 0.1).max(0.0));
                                self.preview_time = self.trim_start;
                                self.needs_scrub_fetch = true;
                            }
                            TimelineDrag::TrimEnd => {
                                self.trim_end = t.max((self.trim_start + 0.1).min(dur));
                                self.preview_time = self.trim_end;
                                self.needs_scrub_fetch = true;
                            }
                            TimelineDrag::TrimWindow => {
                                let len = (self.trim_end - self.trim_start).max(0.1);
                                let new_start =
                                    (t - self.drag_window_offset_secs).clamp(0.0, (dur - len).max(0.0));
                                self.trim_start = new_start;
                                self.trim_end = (new_start + len).min(dur);
                                self.preview_time = self.trim_start;
                                self.needs_scrub_fetch = true;
                            }
                            TimelineDrag::Playhead => {
                                self.preview_time = t;
                                self.needs_scrub_fetch = true;
                            }
                            TimelineDrag::None => {}
                        }
                    }
                }

                if resp.drag_stopped() && self.active_drag != TimelineDrag::None {
                    self.active_drag = TimelineDrag::None;
                    if self.resume_after_drag {
                        self.resume_after_drag = false;
                        self.start_playback_at(self.preview_time, ui.ctx());
                    } else {
                        self.request_frame_async(ui.ctx());
                    }
                }

                // Recompute positions after interaction
                let start_x = bar_left + bar_w * (self.trim_start / dur) as f32;
                let end_x = bar_left + bar_w * (self.trim_end / dur) as f32;
                let play_x = bar_left + bar_w * (self.preview_time / dur) as f32;

                let p = ui.painter();

                // Subtle ruler baseline
                p.line_segment(
                    [
                        Pos2::new(bar_left, ruler_rect.bottom() - 2.0),
                        Pos2::new(bar_right, ruler_rect.bottom() - 2.0),
                    ],
                    Stroke::new(1.0_f32, theme::BORDER_SUBTLE),
                );

                // Background track
                p.rect_filled(track_rect, Rounding::same(7.0), theme::BG_ELEVATED);
                p.rect_stroke(
                    track_rect,
                    Rounding::same(7.0),
                    Stroke::new(1.0_f32, theme::BORDER),
                );

                // Active trim region
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

                // Left & right wide draggable trim handles with grip lines
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

                draw_trim_handle(start_x, hovered_target == TimelineDrag::TrimStart);
                draw_trim_handle(end_x, hovered_target == TimelineDrag::TrimEnd);

                // White playhead vertical line + prominent top flag in ruler
                p.line_segment(
                    [
                        Pos2::new(play_x, ruler_rect.top() + 13.0),
                        Pos2::new(play_x, track_rect.bottom()),
                    ],
                    Stroke::new(2.0_f32, Color32::WHITE),
                );
                let flag_pts = vec![
                    Pos2::new(play_x - 7.0, ruler_rect.top() + 2.0),
                    Pos2::new(play_x + 7.0, ruler_rect.top() + 2.0),
                    Pos2::new(play_x + 7.0, ruler_rect.top() + 11.0),
                    Pos2::new(play_x, ruler_rect.top() + 17.0),
                    Pos2::new(play_x - 7.0, ruler_rect.top() + 11.0),
                ];
                let flag_stroke = if hovered_target == TimelineDrag::Playhead {
                    Stroke::new(1.5_f32, theme::ACCENT)
                } else {
                    Stroke::NONE
                };
                p.add(egui::Shape::convex_polygon(flag_pts, Color32::WHITE, flag_stroke));
            });

        // Modal confirmation dialog for moving video to Trash
        if self.confirm_delete_open {
            let screen = ui.ctx().screen_rect();
            ui.ctx()
                .layer_painter(egui::LayerId::new(
                    egui::Order::Middle,
                    egui::Id::new("confirm_delete_backdrop"),
                ))
                .rect_filled(screen, 0.0, Color32::from_black_alpha(155));

            let filename = self
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("видео")
                .to_string();

            egui::Window::new("confirm_delete_video")
                .order(egui::Order::Foreground)
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .frame(
                    egui::Frame::none()
                        .fill(theme::BG_SURFACE)
                        .stroke(Stroke::new(1.0_f32, theme::BORDER))
                        .rounding(Rounding::same(14.0))
                        .inner_margin(egui::Margin::same(22.0)),
                )
                .show(ui.ctx(), |ui| {
                    ui.set_max_width(360.0);
                    ui.label(
                        egui::RichText::new("Удалить видео в корзину?")
                            .size(16.0)
                            .color(theme::TEXT_PRIMARY)
                            .strong(),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(&filename)
                            .size(13.0)
                            .color(theme::TEXT_PRIMARY),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "Файл переместится в корзину Windows — при желании его можно восстановить.",
                        )
                        .size(12.0)
                        .color(theme::TEXT_SECONDARY),
                    );
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if theme::subtle_button(ui, "Отмена").clicked() {
                            self.confirm_delete_open = false;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::danger_button(ui, "Удалить в корзину").clicked() {
                                self.confirm_delete_open = false;
                                if let Some(path) = self.path.clone() {
                                    self.stop_playback();
                                    match media::move_to_trash(&path) {
                                        Ok(()) => {
                                            self.clear();
                                            action = VideoCanvasAction::Deleted(filename);
                                        }
                                        Err(err) => {
                                            action = VideoCanvasAction::DeleteError(err);
                                        }
                                    }
                                }
                            }
                        });
                    });
                });
        }

        action
    }

    pub fn render_inspector(&mut self, ui: &mut egui::Ui, is_busy: bool) -> Option<VideoExportJob> {
        let mut job = None;
        let dur = self.info.duration_secs.max(0.1);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .max_height((ui.available_height() - 94.0).max(120.0))
            .show(ui, |ui| {
                // Trim card
                theme::section_card(ui, "ОБРЕЗКА ПО ВРЕМЕНИ", |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Начало")
                                .size(12.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::editable_time_field(
                                ui,
                                "video_trim_start",
                                &mut self.trim_start,
                                0.0,
                                (self.trim_end - 0.1).max(0.0),
                            ) {
                                self.stop_playback();
                                self.preview_time = self.trim_start;
                                self.request_frame_async(ui.ctx());
                            }
                        });
                    });
                    let r_start = ui.add(
                        egui::Slider::new(&mut self.trim_start, 0.0..=(dur - 0.1).max(0.0))
                            .show_value(false),
                    );
                    if r_start.drag_started() {
                        self.resume_after_drag = self.is_playing();
                    }
                    if r_start.changed() {
                        self.stop_playback();
                        if self.trim_start >= self.trim_end - 0.1 {
                            self.trim_end = (self.trim_start + 0.1).min(dur);
                        }
                        self.preview_time = self.trim_start;
                        self.needs_scrub_fetch = true;
                    }
                    if r_start.drag_stopped() {
                        if self.resume_after_drag {
                            self.resume_after_drag = false;
                            self.start_playback_at(self.preview_time, ui.ctx());
                        } else {
                            self.request_frame_async(ui.ctx());
                        }
                    }

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Конец")
                                .size(12.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::editable_time_field(
                                ui,
                                "video_trim_end",
                                &mut self.trim_end,
                                (self.trim_start + 0.1).min(dur),
                                dur,
                            ) {
                                self.stop_playback();
                                self.preview_time = self.trim_end;
                                self.request_frame_async(ui.ctx());
                            }
                        });
                    });
                    let r_end =
                        ui.add(egui::Slider::new(&mut self.trim_end, 0.1..=dur).show_value(false));
                    if r_end.drag_started() {
                        self.resume_after_drag = self.is_playing();
                    }
                    if r_end.changed() {
                        self.stop_playback();
                        if self.trim_end <= self.trim_start + 0.1 {
                            self.trim_start = (self.trim_end - 0.1).max(0.0);
                        }
                        self.preview_time = self.trim_end;
                        self.needs_scrub_fetch = true;
                    }
                    if r_end.drag_stopped() {
                        if self.resume_after_drag {
                            self.resume_after_drag = false;
                            self.start_playback_at(self.preview_time, ui.ctx());
                        } else {
                            self.request_frame_async(ui.ctx());
                        }
                    }
                });

                ui.add_space(6.0);

                // Audio mode + volume card
                theme::section_card(ui, "ЗВУК", |ui| {
                    ui.horizontal(|ui| {
                        for mode in [
                            VideoAudioMode::Keep,
                            VideoAudioMode::Mute,
                            VideoAudioMode::ExtractMp3,
                        ] {
                            if theme::pill_choice(ui, self.audio_mode == mode, mode.label()).clicked()
                                && self.audio_mode != mode
                            {
                                self.audio_mode = mode;
                                self.sync_live_audio();
                            }
                        }
                    });

                    if self.audio_mode != VideoAudioMode::Mute {
                        ui.add_space(8.0);
                        let db_text = if self.volume_pct <= 0.5 {
                            "−∞ дБ".to_string()
                        } else {
                            let db = 20.0 * (self.volume_pct / 100.0).log10();
                            format!("{:+.1} дБ", db)
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("Громкость: {:.0}%", self.volume_pct))
                                    .size(12.0)
                                    .color(theme::TEXT_PRIMARY),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    egui::RichText::new(db_text)
                                        .size(11.5)
                                        .color(theme::TEXT_MUTED),
                                );
                            });
                        });
                        if theme::volume_slider_with_steps(ui, &mut self.volume_pct) {
                            self.sync_live_audio();
                        }
                    }

                    if self.audio_mode == VideoAudioMode::ExtractMp3 {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "Примерный размер MP3: ~{}",
                                theme::format_bytes(self.estimate_output_bytes())
                            ))
                            .size(12.0)
                            .color(theme::TEXT_SECONDARY),
                        );
                    }
                });

                if self.audio_mode != VideoAudioMode::ExtractMp3 {
                    ui.add_space(6.0);

                    // Transform card (applied live on GPU without interrupting playback/audio)
                    theme::section_card(ui, "ПОВОРОТ И ОТРАЖЕНИЕ", |ui| {
                        ui.scope(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.spacing_mut().button_padding = Vec2::new(6.0, 4.0);
                            let w = ((ui.available_width() - 6.0) / 2.0).floor();
                            ui.horizontal(|ui| {
                                if ui
                                    .add_sized(
                                        Vec2::new(w, 26.0),
                                        egui::Button::new("↺  90° влево")
                                            .fill(theme::BG_ELEVATED)
                                            .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                            .rounding(Rounding::same(8.0)),
                                    )
                                    .clicked()
                                {
                                    self.rotation_deg = (self.rotation_deg + 270) % 360;
                                }
                                if ui
                                    .add_sized(
                                        Vec2::new(w, 26.0),
                                        egui::Button::new("↻  90° вправо")
                                            .fill(theme::BG_ELEVATED)
                                            .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                            .rounding(Rounding::same(8.0)),
                                    )
                                    .clicked()
                                {
                                    self.rotation_deg = (self.rotation_deg + 90) % 360;
                                }
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add_sized(Vec2::new(w, 26.0), |ui: &mut egui::Ui| {
                                        theme::pill_choice(ui, self.flip_h, "⇄  Горизонтально")
                                    })
                                    .clicked()
                                {
                                    self.flip_h = !self.flip_h;
                                }
                                if ui
                                    .add_sized(Vec2::new(w, 26.0), |ui: &mut egui::Ui| {
                                        theme::pill_choice(ui, self.flip_v, "⇅  Вертикально")
                                    })
                                    .clicked()
                                {
                                    self.flip_v = !self.flip_v;
                                }
                            });
                        });
                    });

                    ui.add_space(6.0);

                    // Compression card
                    theme::section_card(ui, "СЖАТИЕ И КАЧЕСТВО", |ui| {
                        ui.scope(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.spacing_mut().button_padding = Vec2::new(7.0, 5.0);
                            ui.horizontal_wrapped(|ui| {
                                for q in [
                                    VideoQuality::Original,
                                    VideoQuality::High,
                                    VideoQuality::Balanced,
                                    VideoQuality::Compact,
                                ] {
                                    if theme::pill_choice(ui, self.quality == q, q.label()).clicked()
                                    {
                                        self.quality = q;
                                    }
                                }
                            });
                        });

                        ui.add_space(6.0);
                        let est = self.estimate_output_bytes();
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Примерный размер: ~{}",
                                    theme::format_bytes(est)
                                ))
                                .size(12.0)
                                .color(theme::TEXT_PRIMARY),
                            );
                            if self.info.file_size > 0 {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "(исходный: {})",
                                        theme::format_bytes(self.info.file_size)
                                    ))
                                    .size(11.5)
                                    .color(theme::TEXT_MUTED),
                                );
                            }
                        });
                    });

                    ui.add_space(6.0);

                    // Format card
                    theme::section_card(ui, "ФОРМАТ", |ui| {
                        ui.horizontal(|ui| {
                            for fmt in [
                                VideoFormat::Mp4,
                                VideoFormat::Mov,
                                VideoFormat::Webm,
                                VideoFormat::Gif,
                            ] {
                                if theme::pill_choice(ui, self.format == fmt, fmt.label()).clicked()
                                {
                                    self.format = fmt;
                                }
                            }
                        });
                    });
                }

                ui.add_space(8.0);
                if theme::subtle_button(ui, "Сбросить настройки").clicked() {
                    self.reset_edits(ui.ctx());
                }
                ui.add_space(6.0);
            });

        ui.add_space(6.0);
        ui.checkbox(
            &mut self.delete_original_after_export,
            "Удалить оригинал после сохранения",
        )
        .on_hover_text("Исходный файл переместится в корзину после успешного сохранения");
        ui.add_space(6.0);

        let btn_text = if is_busy {
            "Обработка видео..."
        } else if self.audio_mode == VideoAudioMode::ExtractMp3 {
            "Сохранить звук (MP3)"
        } else {
            "Экспортировать видео"
        };

        if let Some(picked) = self.save_dialog.as_ref().and_then(|d| d.poll()) {
            self.save_dialog = None;
            if let (Some(input), Some(output)) = (self.path.clone(), picked) {
                let source_ext = input
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let target_video_kbps =
                    compute_target_video_kbps(&self.info, self.quality, self.format);
                job = Some(VideoExportJob {
                    input,
                    output,
                    trim_start: self.trim_start,
                    trim_end: self.trim_end,
                    rotation_deg: self.rotation_deg,
                    flip_h: self.flip_h,
                    flip_v: self.flip_v,
                    quality: self.quality,
                    audio_mode: self.audio_mode,
                    volume_pct: self.volume_pct,
                    format: self.format,
                    delete_original_after_export: self.delete_original_after_export,
                    source_video_codec: self.info.codec.clone(),
                    source_audio_codec: self.info.audio_codec.clone(),
                    has_audio: self.info.has_audio,
                    source_ext,
                    target_video_kbps,
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

pub fn compute_target_video_kbps(
    info: &MediaInfo,
    quality: VideoQuality,
    format: VideoFormat,
) -> Option<u32> {
    if quality == VideoQuality::Original {
        return None;
    }

    let pixels = (info.width.max(320) as f64) * (info.height.max(240) as f64);
    let res_factor = (pixels / (1920.0 * 1080.0)).powf(0.75).clamp(0.15, 3.5);
    let fps = if info.fps > 1.0 { info.fps } else { 30.0 };
    let fps_factor = (fps / 30.0).clamp(0.75, 2.0).powf(0.45);

    let base = match quality {
        VideoQuality::Original => unreachable!(),
        VideoQuality::High => 5500.0,
        VideoQuality::Balanced => 2600.0,
        VideoQuality::Compact => 1100.0,
    };

    let mut kbps = base * res_factor * fps_factor;

    let orig_total_kbps = (info.file_size as f64 * 8.0) / (info.duration_secs.max(0.1) * 1000.0);
    if orig_total_kbps > 150.0 {
        let orig_video_kbps =
            (orig_total_kbps - if info.has_audio { 192.0 } else { 0.0 }).max(150.0);
        let cap = match quality {
            VideoQuality::High => 0.78,
            VideoQuality::Balanced => 0.48,
            VideoQuality::Compact => 0.25,
            VideoQuality::Original => 1.0,
        };
        kbps = kbps.min(orig_video_kbps * cap);
    }

    if format == VideoFormat::Webm {
        kbps *= 0.80;
    }

    Some(kbps.clamp(250.0, 50_000.0).round() as u32)
}

pub fn run_video_export(
    job: &VideoExportJob,
    progress: Arc<AtomicU32>,
) -> Result<VideoExportResult, String> {
    let saved_path = execute_export_ffmpeg(job, progress)?;

    let mut deleted_original = None;
    let mut warning = None;

    if job.delete_original_after_export {
        let out_ok = std::fs::metadata(&saved_path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
        let same_file = media::is_same_file(&job.input, &saved_path);

        if out_ok && !same_file {
            match media::move_to_trash(&job.input) {
                Ok(()) => deleted_original = Some(job.input.clone()),
                Err(err) => {
                    warning = Some(format!("Видео сохранено, но оригинал не удалён: {}", err));
                }
            }
        } else if same_file {
            warning = Some("Выбран тот же файл — оригинал не удалялся".to_string());
        }
    }

    Ok(VideoExportResult {
        saved_path,
        deleted_original,
        warning,
    })
}

fn execute_export_ffmpeg(
    job: &VideoExportJob,
    progress: Arc<AtomicU32>,
) -> Result<PathBuf, String> {
    let overwrite_in_place = media::is_same_file(&job.input, &job.output);
    let actual_out = if overwrite_in_place {
        media::temp_output_path(&job.output)
    } else {
        job.output.clone()
    };

    let duration = (job.trim_end - job.trim_start).max(0.1);
    let has_volume_change = (job.volume_pct - 100.0).abs() > 0.5;
    let volume_filter = format!("volume={:.3}", (job.volume_pct / 100.0).max(0.0));

    let res = if job.audio_mode == VideoAudioMode::ExtractMp3 {
        if !job.has_audio {
            return Err("В этом видео нет звуковой дорожки".to_string());
        }
        let mut cmd = media::hidden_command("ffmpeg");
        cmd.arg("-y");
        if job.trim_start > 0.01 {
            cmd.args(["-ss", &format!("{:.3}", job.trim_start)]);
        }
        cmd.arg("-i").arg(&job.input);
        cmd.args(["-t", &format!("{:.3}", duration), "-vn"]);
        if has_volume_change {
            cmd.args(["-af", &volume_filter]);
        }
        cmd.args(["-c:a", "libmp3lame", "-q:a", "2"]);
        media::execute_ffmpeg_with_progress(cmd, &actual_out, duration, progress)
    } else if job.format == VideoFormat::Gif {
        let mut cmd = media::hidden_command("ffmpeg");
        cmd.arg("-y");
        if job.trim_start > 0.01 {
            cmd.args(["-ss", &format!("{:.3}", job.trim_start)]);
        }
        cmd.arg("-i").arg(&job.input);
        cmd.args(["-t", &format!("{:.3}", duration)]);
        let mut vf_parts: Vec<String> = Vec::new();
        media::push_video_transform_filters(
            &mut vf_parts,
            job.rotation_deg,
            job.flip_h,
            job.flip_v,
        );
        vf_parts.push("fps=12,scale='min(640,iw)':-2:flags=lanczos".to_string());
        cmd.args(["-vf", &vf_parts.join(",")]);
        media::execute_ffmpeg_with_progress(cmd, &actual_out, duration, progress)
    } else if job.format == VideoFormat::Webm {
        let cmd = build_webm_cmd(job, duration, has_volume_change, &volume_filter);
        media::execute_ffmpeg_with_progress(cmd, &actual_out, duration, progress)
    } else {
        // MP4 / MOV: 1) instant stream-copy when lossless/untransformed, 2) GPU encoder, 3) CPU libx264 fallback
        let can_copy_video = job.quality == VideoQuality::Original
            && job.rotation_deg.is_multiple_of(360)
            && !job.flip_h
            && !job.flip_v
            && job.trim_start <= 0.05
            && matches!(job.source_video_codec.as_str(), "H264" | "HEVC")
            && matches!(job.source_ext.as_str(), "mp4" | "mov" | "mkv" | "m4v");

        if can_copy_video {
            let cmd = build_mp4_stream_copy_cmd(job, duration, has_volume_change, &volume_filter);
            match media::execute_ffmpeg_with_progress(
                cmd,
                &actual_out,
                duration,
                Arc::clone(&progress),
            ) {
                Ok(p) => Ok(p),
                Err(_) => {
                    let _ = std::fs::remove_file(&actual_out);
                    progress.store(0.0_f32.to_bits(), Ordering::Relaxed);
                    encode_mp4_with_hw_fallback(
                        job,
                        duration,
                        has_volume_change,
                        &volume_filter,
                        &actual_out,
                        progress,
                    )
                }
            }
        } else {
            encode_mp4_with_hw_fallback(
                job,
                duration,
                has_volume_change,
                &volume_filter,
                &actual_out,
                progress,
            )
        }
    };

    if overwrite_in_place {
        match res {
            Ok(_) => {
                media::finalize_temp_replace(&actual_out, &job.output)?;
                Ok(job.output.clone())
            }
            Err(e) => {
                let _ = std::fs::remove_file(&actual_out);
                Err(e)
            }
        }
    } else {
        res
    }
}

fn encode_mp4_with_hw_fallback(
    job: &VideoExportJob,
    duration: f64,
    has_volume_change: bool,
    volume_filter: &str,
    actual_out: &Path,
    progress: Arc<AtomicU32>,
) -> Result<PathBuf, String> {
    if let Some(hw_enc) = media::detect_hw_h264_encoder() {
        let hw_cmd =
            build_mp4_encode_cmd(job, duration, has_volume_change, volume_filter, Some(hw_enc));
        match media::execute_ffmpeg_with_progress(
            hw_cmd,
            actual_out,
            duration,
            Arc::clone(&progress),
        ) {
            Ok(p) => return Ok(p),
            Err(_) => {
                let _ = std::fs::remove_file(actual_out);
                progress.store(0.0_f32.to_bits(), Ordering::Relaxed);
            }
        }
    }
    let cpu_cmd = build_mp4_encode_cmd(job, duration, has_volume_change, volume_filter, None);
    media::execute_ffmpeg_with_progress(cpu_cmd, actual_out, duration, progress)
}

fn push_mp4_audio_args(
    cmd: &mut std::process::Command,
    job: &VideoExportJob,
    has_volume_change: bool,
    volume_filter: &str,
) {
    if !job.has_audio || job.audio_mode == VideoAudioMode::Mute {
        cmd.arg("-an");
        return;
    }
    if !has_volume_change && job.source_audio_codec == "AAC" {
        cmd.args(["-c:a", "copy"]);
    } else {
        if has_volume_change {
            cmd.args(["-af", volume_filter]);
        }
        cmd.args(["-c:a", "aac", "-b:a", "192k"]);
    }
}

fn build_mp4_stream_copy_cmd(
    job: &VideoExportJob,
    duration: f64,
    has_volume_change: bool,
    volume_filter: &str,
) -> std::process::Command {
    let mut cmd = media::hidden_command("ffmpeg");
    cmd.arg("-y").arg("-i").arg(&job.input);
    cmd.args(["-t", &format!("{:.3}", duration), "-c:v", "copy"]);
    push_mp4_audio_args(&mut cmd, job, has_volume_change, volume_filter);
    cmd.args(["-movflags", "+faststart"]);
    cmd
}

fn build_mp4_encode_cmd(
    job: &VideoExportJob,
    duration: f64,
    has_volume_change: bool,
    volume_filter: &str,
    hw_enc: Option<&str>,
) -> std::process::Command {
    let mut cmd = media::hidden_command("ffmpeg");
    cmd.arg("-y");
    if job.trim_start > 0.01 {
        cmd.args(["-ss", &format!("{:.3}", job.trim_start)]);
    }
    cmd.arg("-i").arg(&job.input);
    cmd.args(["-t", &format!("{:.3}", duration)]);

    let mut vf_parts: Vec<String> = Vec::new();
    media::push_video_transform_filters(
        &mut vf_parts,
        job.rotation_deg,
        job.flip_h,
        job.flip_v,
    );
    vf_parts.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string());
    cmd.args(["-vf", &vf_parts.join(",")]);

    if let Some(kbps) = job.target_video_kbps {
        let maxrate = ((kbps as f64 * 1.25).round() as u32).max(kbps + 100);
        let bufsize = kbps * 2;
        let bv_arg = format!("{}k", kbps);
        let maxrate_arg = format!("{}k", maxrate);
        let bufsize_arg = format!("{}k", bufsize);

        match hw_enc {
            Some("h264_nvenc") => {
                cmd.args([
                    "-c:v",
                    "h264_nvenc",
                    "-preset",
                    "p4",
                    "-rc",
                    "vbr",
                    "-b:v",
                    &bv_arg,
                    "-maxrate",
                    &maxrate_arg,
                    "-bufsize",
                    &bufsize_arg,
                    "-pix_fmt",
                    "yuv420p",
                ]);
            }
            Some("h264_qsv") => {
                cmd.args([
                    "-c:v",
                    "h264_qsv",
                    "-b:v",
                    &bv_arg,
                    "-maxrate",
                    &maxrate_arg,
                    "-bufsize",
                    &bufsize_arg,
                    "-pix_fmt",
                    "nv12",
                ]);
            }
            Some("h264_amf") => {
                cmd.args([
                    "-c:v",
                    "h264_amf",
                    "-quality",
                    "balanced",
                    "-rc",
                    "vbr_peak",
                    "-b:v",
                    &bv_arg,
                    "-maxrate",
                    &maxrate_arg,
                    "-bufsize",
                    &bufsize_arg,
                    "-pix_fmt",
                    "yuv420p",
                ]);
            }
            _ => {
                cmd.args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "fast",
                    "-threads",
                    "0",
                    "-b:v",
                    &bv_arg,
                    "-maxrate",
                    &maxrate_arg,
                    "-bufsize",
                    &bufsize_arg,
                    "-pix_fmt",
                    "yuv420p",
                ]);
            }
        }
    } else {
        let cq = match job.quality {
            VideoQuality::Original => "19",
            VideoQuality::High => "22",
            VideoQuality::Balanced => "27",
            VideoQuality::Compact => "33",
        };

        match hw_enc {
            Some("h264_nvenc") => {
                cmd.args([
                    "-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-cq", cq, "-b:v", "0",
                    "-pix_fmt", "yuv420p",
                ]);
            }
            Some("h264_qsv") => {
                cmd.args([
                    "-c:v", "h264_qsv", "-global_quality", cq, "-pix_fmt", "nv12",
                ]);
            }
            Some("h264_amf") => {
                cmd.args([
                    "-c:v", "h264_amf", "-quality", "balanced", "-rc", "cqp", "-qp_i", cq, "-qp_p",
                    cq, "-pix_fmt", "yuv420p",
                ]);
            }
            _ => {
                cmd.args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "fast",
                    "-threads",
                    "0",
                    "-crf",
                    job.quality.crf(),
                    "-pix_fmt",
                    "yuv420p",
                ]);
            }
        }
    }

    push_mp4_audio_args(&mut cmd, job, has_volume_change, volume_filter);
    cmd.args(["-movflags", "+faststart"]);
    cmd
}

fn build_webm_cmd(
    job: &VideoExportJob,
    duration: f64,
    has_volume_change: bool,
    volume_filter: &str,
) -> std::process::Command {
    let mut cmd = media::hidden_command("ffmpeg");
    cmd.arg("-y");
    if job.trim_start > 0.01 {
        cmd.args(["-ss", &format!("{:.3}", job.trim_start)]);
    }
    cmd.arg("-i").arg(&job.input);
    cmd.args(["-t", &format!("{:.3}", duration)]);

    let mut vf_parts: Vec<String> = Vec::new();
    media::push_video_transform_filters(
        &mut vf_parts,
        job.rotation_deg,
        job.flip_h,
        job.flip_v,
    );
    vf_parts.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string());
    cmd.args(["-vf", &vf_parts.join(",")]);
    cmd.args([
        "-c:v",
        "libvpx-vp9",
        "-row-mt",
        "1",
        "-cpu-used",
        "4",
        "-deadline",
        "good",
        "-threads",
        "0",
    ]);
    if let Some(kbps) = job.target_video_kbps {
        let maxrate = ((kbps as f64 * 1.25).round() as u32).max(kbps + 100);
        let bufsize = kbps * 2;
        let bv_arg = format!("{}k", kbps);
        let maxrate_arg = format!("{}k", maxrate);
        let bufsize_arg = format!("{}k", bufsize);
        cmd.args([
            "-b:v",
            &bv_arg,
            "-maxrate",
            &maxrate_arg,
            "-bufsize",
            &bufsize_arg,
        ]);
    } else {
        cmd.args(["-crf", job.quality.crf(), "-b:v", "0"]);
    }

    if !job.has_audio || job.audio_mode == VideoAudioMode::Mute {
        cmd.arg("-an");
    } else {
        if has_volume_change {
            cmd.args(["-af", volume_filter]);
        }
        cmd.args(["-c:a", "libopus", "-b:a", "128k"]);
    }
    cmd
}

fn paint_transformed_texture(
    painter: &egui::Painter,
    tex_id: egui::TextureId,
    rect: Rect,
    rotation_deg: u32,
    flip_h: bool,
    flip_v: bool,
) {
    let uv_for = |sx: f32, sy: f32| -> Pos2 {
        let fx = if flip_h { 1.0 - sx } else { sx };
        let fy = if flip_v { 1.0 - sy } else { sy };
        let (u, v) = match rotation_deg % 360 {
            90 => (fy, 1.0 - fx),
            180 => (1.0 - fx, 1.0 - fy),
            270 => (1.0 - fy, fx),
            _ => (fx, fy),
        };
        Pos2::new(u, v)
    };

    let mut mesh = egui::Mesh::with_texture(tex_id);
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.left_top(),
        uv: uv_for(0.0, 0.0),
        color: Color32::WHITE,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.right_top(),
        uv: uv_for(1.0, 0.0),
        color: Color32::WHITE,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.right_bottom(),
        uv: uv_for(1.0, 1.0),
        color: Color32::WHITE,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.left_bottom(),
        uv: uv_for(0.0, 1.0),
        color: Color32::WHITE,
    });
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

fn truncate_display(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{}…", head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dvr_info() -> MediaInfo {
        MediaInfo {
            duration_secs: 150.0,
            width: 1920,
            height: 1080,
            fps: 60.0,
            codec: "H264".to_string(),
            audio_codec: "AAC".to_string(),
            has_audio: true,
            bitrate_kbps: 31_000,
            file_size: (583.1_f64 * 1024.0 * 1024.0) as u64,
        }
    }

    #[test]
    fn target_bitrate_scales_monotonically_with_quality() {
        let info = sample_dvr_info();
        assert_eq!(
            compute_target_video_kbps(&info, VideoQuality::Original, VideoFormat::Mp4),
            None
        );

        let high = compute_target_video_kbps(&info, VideoQuality::High, VideoFormat::Mp4).unwrap();
        let bal = compute_target_video_kbps(&info, VideoQuality::Balanced, VideoFormat::Mp4).unwrap();
        let cmp = compute_target_video_kbps(&info, VideoQuality::Compact, VideoFormat::Mp4).unwrap();

        assert!(high > bal && bal > cmp);
        // 1080p60 High should be around 7500 kbps (~5500 * 1.366)
        assert!((7000..=8000).contains(&high), "high kbps = {}", high);
    }

    #[test]
    fn estimate_matches_target_bitrate_for_dvr_clip() {
        let mut ed = VideoEditor::default();
        ed.info = sample_dvr_info();
        ed.trim_start = 0.0;
        ed.trim_end = ed.info.duration_secs;
        ed.format = VideoFormat::Mp4;
        ed.audio_mode = VideoAudioMode::Keep;

        ed.quality = VideoQuality::Original;
        let est_orig = ed.estimate_output_bytes();
        assert_eq!(est_orig, ed.info.file_size);

        ed.quality = VideoQuality::High;
        let est_high = ed.estimate_output_bytes();

        ed.quality = VideoQuality::Balanced;
        let est_bal = ed.estimate_output_bytes();

        ed.quality = VideoQuality::Compact;
        let est_cmp = ed.estimate_output_bytes();

        assert!(est_orig > est_high && est_high > est_bal && est_bal > est_cmp);
        // For 150s @ ~7512 kbps video + 192 kbps audio, estimate should be ~140-150 MB
        let mb = est_high as f64 / (1024.0 * 1024.0);
        assert!(mb > 130.0 && mb < 160.0, "est_high MB = {:.1}", mb);
    }
}
