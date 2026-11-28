use crate::theme;
use eframe::egui::{
    self, Color32, CursorIcon, Pos2, Rect, Rounding, Stroke, TextureHandle, TextureOptions, Vec2,
};
use image::{DynamicImage, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PhotoFormat {
    Png,
    Jpeg,
    Webp,
}

impl PhotoFormat {
    pub fn ext(&self) -> &'static str {
        match self {
            PhotoFormat::Png => "png",
            PhotoFormat::Jpeg => "jpg",
            PhotoFormat::Webp => "webp",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PhotoFormat::Png => "PNG",
            PhotoFormat::Jpeg => "JPEG",
            PhotoFormat::Webp => "WEBP",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CropAspect {
    Free,
    Square,
    W16H9,
    W4H3,
    W9H16,
}

impl CropAspect {
    pub fn label(&self) -> &'static str {
        match self {
            CropAspect::Free => "Свободно",
            CropAspect::Square => "1:1",
            CropAspect::W16H9 => "16:9",
            CropAspect::W4H3 => "4:3",
            CropAspect::W9H16 => "9:16",
        }
    }

    pub fn ratio(&self) -> Option<f32> {
        match self {
            CropAspect::Free => None,
            CropAspect::Square => Some(1.0),
            CropAspect::W16H9 => Some(16.0 / 9.0),
            CropAspect::W4H3 => Some(4.0 / 3.0),
            CropAspect::W9H16 => Some(9.0 / 16.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CropDrag {
    None,
    Move,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Left,
    Right,
    Top,
    Bottom,
}

pub struct PhotoEditor {
    pub path: Option<PathBuf>,
    pub file_size: u64,
    original: Option<Arc<DynamicImage>>,
    preview_base: Option<DynamicImage>,
    preview_texture: Option<TextureHandle>,
    dirty: bool,

    // Edits
    pub rotation_steps: u8, // 0, 1 (90°), 2 (180°), 3 (270°)
    pub flip_h: bool,
    pub flip_v: bool,

    pub brightness: i32, // -100..=100
    pub contrast: f32,   // -80..=80
    pub saturation: f32, // 0.0..=2.0 (1.0 = normal)
    pub grayscale: bool,

    // Interactive framing / crop (normalized 0.0..=1.0)
    pub crop_mode: bool,
    pub crop_aspect: CropAspect,
    pub crop_x0: f32,
    pub crop_y0: f32,
    pub crop_x1: f32,
    pub crop_y1: f32,
    active_drag: CropDrag,
    drag_aspect_ratio: Option<f32>,

    // Resize
    pub custom_resize: bool,
    pub lock_aspect: bool,
    pub target_width: u32,
    pub target_height: u32,

    // Export
    pub format: PhotoFormat,
    pub jpeg_quality: u8,
    save_dialog: Option<crate::media::PendingFileDialog>,
    history: crate::media::UndoHistory<PhotoSnapshot>,
    frame_snapshot: Option<PhotoSnapshot>,
}

#[derive(Clone)]
pub struct PhotoExportJob {
    pub original: Arc<DynamicImage>,
    pub output: PathBuf,
    pub rotation_steps: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    pub brightness: i32,
    pub contrast: f32,
    pub saturation: f32,
    pub grayscale: bool,
    pub is_cropped: bool,
    pub crop_x0: f32,
    pub crop_y0: f32,
    pub crop_x1: f32,
    pub crop_y1: f32,
    pub custom_resize: bool,
    pub target_width: u32,
    pub target_height: u32,
    pub format: PhotoFormat,
    pub jpeg_quality: u8,
}

#[derive(Clone, Copy)]
struct PhotoTransformParams {
    rotation_steps: u8,
    flip_h: bool,
    flip_v: bool,
    brightness: i32,
    contrast: f32,
    saturation: f32,
    grayscale: bool,
    is_cropped: bool,
    crop_x0: f32,
    crop_y0: f32,
    crop_x1: f32,
    crop_y1: f32,
    custom_resize: bool,
    target_width: u32,
    target_height: u32,
}

#[derive(Clone, PartialEq)]
pub struct PhotoSnapshot {
    rotation_steps: u8,
    flip_h: bool,
    flip_v: bool,
    brightness: i32,
    contrast: f32,
    saturation: f32,
    grayscale: bool,
    crop_mode: bool,
    crop_aspect: CropAspect,
    crop_x0: f32,
    crop_y0: f32,
    crop_x1: f32,
    crop_y1: f32,
    custom_resize: bool,
    lock_aspect: bool,
    target_width: u32,
    target_height: u32,
    format: PhotoFormat,
    jpeg_quality: u8,
}

impl Default for PhotoEditor {
    fn default() -> Self {
        Self {
            path: None,
            file_size: 0,
            original: None,
            preview_base: None,
            preview_texture: None,
            dirty: false,
            rotation_steps: 0,
            flip_h: false,
            flip_v: false,
            brightness: 0,
            contrast: 0.0,
            saturation: 1.0,
            grayscale: false,
            crop_mode: false,
            crop_aspect: CropAspect::Free,
            crop_x0: 0.0,
            crop_y0: 0.0,
            crop_x1: 1.0,
            crop_y1: 1.0,
            active_drag: CropDrag::None,
            drag_aspect_ratio: None,
            custom_resize: false,
            lock_aspect: true,
            target_width: 1920,
            target_height: 1080,
            format: PhotoFormat::Png,
            jpeg_quality: 90,
            save_dialog: None,
            history: crate::media::UndoHistory::default(),
            frame_snapshot: None,
        }
    }
}

impl PhotoEditor {
    pub fn load(&mut self, path: PathBuf) -> Result<(), String> {
        let img = image::open(&path).map_err(|e| format!("Не удалось открыть фото: {}", e))?;
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let (w, h) = img.dimensions();

        // Crisp preview base (max 1920px on longest side)
        let preview = if w > 1920 || h > 1920 {
            img.resize(1920, 1920, image::imageops::FilterType::Lanczos3)
        } else {
            img.clone()
        };

        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format = match ext.as_str() {
            "jpg" | "jpeg" => PhotoFormat::Jpeg,
            "webp" => PhotoFormat::Webp,
            _ => PhotoFormat::Png,
        };

        self.path = Some(path);
        self.file_size = file_size;
        self.original = Some(Arc::new(img));
        self.preview_base = Some(preview);
        self.preview_texture = None;
        self.format = format;
        self.reset_edits();
        self.target_width = w;
        self.target_height = h;
        self.save_dialog = None;
        self.history.clear();
        self.frame_snapshot = None;
        Ok(())
    }

    fn rotate_ccw(&mut self) {
        self.rotation_steps = (self.rotation_steps + 3) % 4;
        if self.custom_resize {
            std::mem::swap(&mut self.target_width, &mut self.target_height);
        }
        self.dirty = true;
    }

    fn rotate_cw(&mut self) {
        self.rotation_steps = (self.rotation_steps + 1) % 4;
        if self.custom_resize {
            std::mem::swap(&mut self.target_width, &mut self.target_height);
        }
        self.dirty = true;
    }

    fn transform_params(&self) -> PhotoTransformParams {
        PhotoTransformParams {
            rotation_steps: self.rotation_steps,
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            brightness: self.brightness,
            contrast: self.contrast,
            saturation: self.saturation,
            grayscale: self.grayscale,
            is_cropped: self.is_cropped(),
            crop_x0: self.crop_x0,
            crop_y0: self.crop_y0,
            crop_x1: self.crop_x1,
            crop_y1: self.crop_y1,
            custom_resize: self.custom_resize,
            target_width: self.target_width,
            target_height: self.target_height,
        }
    }

    pub fn snapshot(&self) -> PhotoSnapshot {
        PhotoSnapshot {
            rotation_steps: self.rotation_steps,
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            brightness: self.brightness,
            contrast: self.contrast,
            saturation: self.saturation,
            grayscale: self.grayscale,
            crop_mode: self.crop_mode,
            crop_aspect: self.crop_aspect,
            crop_x0: self.crop_x0,
            crop_y0: self.crop_y0,
            crop_x1: self.crop_x1,
            crop_y1: self.crop_y1,
            custom_resize: self.custom_resize,
            lock_aspect: self.lock_aspect,
            target_width: self.target_width,
            target_height: self.target_height,
            format: self.format,
            jpeg_quality: self.jpeg_quality,
        }
    }

    pub fn apply_snapshot(&mut self, s: PhotoSnapshot) {
        self.rotation_steps = s.rotation_steps;
        self.flip_h = s.flip_h;
        self.flip_v = s.flip_v;
        self.brightness = s.brightness;
        self.contrast = s.contrast;
        self.saturation = s.saturation;
        self.grayscale = s.grayscale;
        self.crop_mode = s.crop_mode;
        self.crop_aspect = s.crop_aspect;
        self.crop_x0 = s.crop_x0;
        self.crop_y0 = s.crop_y0;
        self.crop_x1 = s.crop_x1;
        self.crop_y1 = s.crop_y1;
        self.custom_resize = s.custom_resize;
        self.lock_aspect = s.lock_aspect;
        self.target_width = s.target_width;
        self.target_height = s.target_height;
        self.format = s.format;
        self.jpeg_quality = s.jpeg_quality;
        self.dirty = true;
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
        let (rot_l, rot_r, fh, fv, crop, esc_enter) = ctx.input(|i| {
            let no_mods = !i.modifiers.ctrl && !i.modifiers.command && !i.modifiers.alt;
            (
                no_mods && i.key_pressed(egui::Key::L),
                no_mods && i.key_pressed(egui::Key::R),
                no_mods && i.key_pressed(egui::Key::H),
                no_mods && i.key_pressed(egui::Key::V),
                no_mods && i.key_pressed(egui::Key::C),
                i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape),
            )
        });
        if rot_l {
            self.rotate_ccw();
        }
        if rot_r {
            self.rotate_cw();
        }
        if fh {
            self.flip_h = !self.flip_h;
            self.dirty = true;
        }
        if fv {
            self.flip_v = !self.flip_v;
            self.dirty = true;
        }
        if crop {
            self.crop_mode = !self.crop_mode;
            self.dirty = true;
        }
        if self.crop_mode && esc_enter {
            self.crop_mode = false;
            self.dirty = true;
        }
    }

    pub fn request_save(&mut self, ctx: &egui::Context) {
        if !self.has_file() || self.save_dialog.is_some() {
            return;
        }
        let default_name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(|s| format!("{}_edited.{}", s, self.format.ext()))
            .unwrap_or_else(|| format!("photo_edited.{}", self.format.ext()));
        let label = self.format.label();
        let ext = self.format.ext();
        self.save_dialog = Some(crate::media::PendingFileDialog::spawn(ctx, move || {
            rfd::FileDialog::new()
                .set_file_name(&default_name)
                .add_filter(label, &[ext])
                .save_file()
        }));
    }

    pub fn reset_crop(&mut self) {
        self.crop_x0 = 0.0;
        self.crop_y0 = 0.0;
        self.crop_x1 = 1.0;
        self.crop_y1 = 1.0;
        self.crop_aspect = CropAspect::Free;
        self.active_drag = CropDrag::None;
        self.dirty = true;
    }

    pub fn reset_edits(&mut self) {
        self.rotation_steps = 0;
        self.flip_h = false;
        self.flip_v = false;
        self.brightness = 0;
        self.contrast = 0.0;
        self.saturation = 1.0;
        self.grayscale = false;
        self.crop_mode = false;
        self.reset_crop();
        self.custom_resize = false;
        if let Some(orig) = &self.original {
            let (w, h) = orig.dimensions();
            self.target_width = w;
            self.target_height = h;
        }
        self.dirty = true;
    }

    pub fn has_file(&self) -> bool {
        self.original.is_some()
    }

    pub fn clear(&mut self) {
        self.path = None;
        self.file_size = 0;
        self.original = None;
        self.preview_base = None;
        self.preview_texture = None;
        self.dirty = false;
        self.crop_mode = false;
        self.active_drag = CropDrag::None;
        self.drag_aspect_ratio = None;
        self.save_dialog = None;
        self.history.clear();
        self.frame_snapshot = None;
    }

    fn is_cropped(&self) -> bool {
        self.crop_x0 > 0.002 || self.crop_y0 > 0.002 || self.crop_x1 < 0.998 || self.crop_y1 < 0.998
    }

    fn rotated_dimensions(&self) -> (u32, u32) {
        let Some(orig) = &self.original else {
            return (1, 1);
        };
        let (ow, oh) = orig.dimensions();
        if self.rotation_steps % 2 == 1 {
            (oh.max(1), ow.max(1))
        } else {
            (ow.max(1), oh.max(1))
        }
    }

    fn effective_dimensions(&self) -> (u32, u32) {
        let (rw, rh) = self.rotated_dimensions();
        let cw = ((rw as f32) * (self.crop_x1 - self.crop_x0).max(0.02)).round() as u32;
        let ch = ((rh as f32) * (self.crop_y1 - self.crop_y0).max(0.02)).round() as u32;
        (cw.max(1), ch.max(1))
    }

    fn apply_aspect_preset(&mut self, aspect: CropAspect) {
        self.crop_aspect = aspect;
        self.crop_mode = true;
        self.dirty = true;

        let Some(target_ratio) = aspect.ratio() else {
            return;
        };

        let (rw, rh) = self.rotated_dimensions();
        let img_aspect = (rw as f32) / (rh as f32);

        let (dx, dy) = if img_aspect > target_ratio {
            (target_ratio / img_aspect, 1.0)
        } else {
            (1.0, img_aspect / target_ratio)
        };

        self.crop_x0 = (1.0 - dx) * 0.5;
        self.crop_x1 = self.crop_x0 + dx;
        self.crop_y0 = (1.0 - dy) * 0.5;
        self.crop_y1 = self.crop_y0 + dy;
    }

    fn rebuild_preview(&mut self, ctx: &egui::Context) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let Some(base) = &self.preview_base else {
            return;
        };

        // In crop mode show the full image so user can adjust the frame freely;
        // outside crop mode show the already-cropped & resized result cleanly!
        let params = self.transform_params();
        let processed = apply_photo_pipeline(base, &params, !self.crop_mode, !self.crop_mode, true);
        let rgba = processed.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        self.preview_texture = Some(ctx.load_texture(
            "photo_preview",
            color_image,
            TextureOptions::LINEAR,
        ));
    }

    fn hit_test_crop(pos: Pos2, crop_rect: Rect) -> CropDrag {
        let corner_r = 14.0;
        let edge_r = 9.0;

        let tl = crop_rect.left_top();
        let tr = crop_rect.right_top();
        let bl = crop_rect.left_bottom();
        let br = crop_rect.right_bottom();

        if pos.distance(tl) <= corner_r {
            return CropDrag::TopLeft;
        }
        if pos.distance(tr) <= corner_r {
            return CropDrag::TopRight;
        }
        if pos.distance(bl) <= corner_r {
            return CropDrag::BottomLeft;
        }
        if pos.distance(br) <= corner_r {
            return CropDrag::BottomRight;
        }

        if (pos.x - crop_rect.left()).abs() <= edge_r
            && pos.y >= crop_rect.top()
            && pos.y <= crop_rect.bottom()
        {
            return CropDrag::Left;
        }
        if (pos.x - crop_rect.right()).abs() <= edge_r
            && pos.y >= crop_rect.top()
            && pos.y <= crop_rect.bottom()
        {
            return CropDrag::Right;
        }
        if (pos.y - crop_rect.top()).abs() <= edge_r
            && pos.x >= crop_rect.left()
            && pos.x <= crop_rect.right()
        {
            return CropDrag::Top;
        }
        if (pos.y - crop_rect.bottom()).abs() <= edge_r
            && pos.x >= crop_rect.left()
            && pos.x <= crop_rect.right()
        {
            return CropDrag::Bottom;
        }

        if crop_rect.contains(pos) {
            return CropDrag::Move;
        }

        CropDrag::None
    }

    fn handle_crop_interaction(&mut self, ui: &mut egui::Ui, canvas_rect: Rect, img_rect: Rect) {
        let min_size = 0.05_f32;
        let img_w = img_rect.width().max(1.0);
        let img_h = img_rect.height().max(1.0);
        let img_aspect = img_w / img_h;

        let crop_rect = Rect::from_min_max(
            Pos2::new(
                img_rect.left() + img_w * self.crop_x0,
                img_rect.top() + img_h * self.crop_y0,
            ),
            Pos2::new(
                img_rect.left() + img_w * self.crop_x1,
                img_rect.top() + img_h * self.crop_y1,
            ),
        );

        let resp = ui.interact(
            canvas_rect,
            ui.id().with("interactive_crop"),
            egui::Sense::click_and_drag(),
        );

        // Update hover cursor
        if let Some(hover_pos) = resp.hover_pos() {
            let mode = if self.active_drag != CropDrag::None {
                self.active_drag
            } else {
                Self::hit_test_crop(hover_pos, crop_rect)
            };
            let cursor = match mode {
                CropDrag::TopLeft | CropDrag::BottomRight => CursorIcon::ResizeNwSe,
                CropDrag::TopRight | CropDrag::BottomLeft => CursorIcon::ResizeNeSw,
                CropDrag::Left | CropDrag::Right => CursorIcon::ResizeHorizontal,
                CropDrag::Top | CropDrag::Bottom => CursorIcon::ResizeVertical,
                CropDrag::Move => {
                    if self.active_drag == CropDrag::Move {
                        CursorIcon::Grabbing
                    } else {
                        CursorIcon::Grab
                    }
                }
                CropDrag::None => CursorIcon::Default,
            };
            ui.ctx().set_cursor_icon(cursor);
        }

        let shift_held = ui.ctx().input(|i| i.modifiers.shift);

        if resp.drag_started() {
            let origin = ui
                .ctx()
                .input(|i| i.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(pos) = origin {
                self.active_drag = Self::hit_test_crop(pos, crop_rect);
                if shift_held && self.crop_aspect.ratio().is_none() {
                    let cur_w = ((self.crop_x1 - self.crop_x0) * img_w).max(1.0);
                    let cur_h = ((self.crop_y1 - self.crop_y0) * img_h).max(1.0);
                    self.drag_aspect_ratio = Some(cur_w / cur_h);
                } else {
                    self.drag_aspect_ratio = None;
                }
            }
        }

        // Single click anywhere outside the crop frame applies and closes framing immediately
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if !crop_rect.expand(12.0).contains(pos) {
                    self.crop_mode = false;
                    self.dirty = true;
                    ui.ctx().request_repaint();
                }
            }
        }

        if resp.dragged() && self.active_drag != CropDrag::None {
            if shift_held && self.crop_aspect.ratio().is_none() {
                if self.drag_aspect_ratio.is_none() {
                    let cur_w = ((self.crop_x1 - self.crop_x0) * img_w).max(1.0);
                    let cur_h = ((self.crop_y1 - self.crop_y0) * img_h).max(1.0);
                    self.drag_aspect_ratio = Some(cur_w / cur_h);
                }
            } else {
                self.drag_aspect_ratio = None;
            }

            let delta = resp.drag_delta();
            let dx = delta.x / img_w;
            let dy = delta.y / img_h;

            match self.active_drag {
                CropDrag::Move => {
                    let w = (self.crop_x1 - self.crop_x0).clamp(0.0, 1.0);
                    let h = (self.crop_y1 - self.crop_y0).clamp(0.0, 1.0);
                    let nx0 = (self.crop_x0 + dx).clamp(0.0, (1.0 - w).max(0.0));
                    let ny0 = (self.crop_y0 + dy).clamp(0.0, (1.0 - h).max(0.0));
                    self.crop_x0 = nx0;
                    self.crop_x1 = (nx0 + w).min(1.0);
                    self.crop_y0 = ny0;
                    self.crop_y1 = (ny0 + h).min(1.0);
                }
                drag => {
                    let locked_ratio = self.crop_aspect.ratio().or(self.drag_aspect_ratio);
                    if let Some(target_ratio) = locked_ratio {
                        let k = (img_aspect / target_ratio).max(0.05);
                        match drag {
                            CropDrag::BottomRight | CropDrag::Right | CropDrag::Bottom => {
                                let mut w = (self.crop_x1 - self.crop_x0 + dx).max(min_size);
                                if drag == CropDrag::Bottom {
                                    let h = (self.crop_y1 - self.crop_y0 + dy).max(min_size);
                                    w = h / k;
                                }
                                let max_w = (1.0 - self.crop_x0)
                                    .min((1.0 - self.crop_y0) / k)
                                    .max(0.01);
                                let min_w = min_size.min(max_w);
                                w = w.clamp(min_w, max_w);
                                self.crop_x1 = (self.crop_x0 + w).min(1.0);
                                self.crop_y1 = (self.crop_y0 + w * k).min(1.0);
                            }
                            CropDrag::TopLeft | CropDrag::Left | CropDrag::Top => {
                                let mut w = (self.crop_x1 - self.crop_x0 - dx).max(min_size);
                                if drag == CropDrag::Top {
                                    let h = (self.crop_y1 - self.crop_y0 - dy).max(min_size);
                                    w = h / k;
                                }
                                let max_w = self.crop_x1.min(self.crop_y1 / k).max(0.01);
                                let min_w = min_size.min(max_w);
                                w = w.clamp(min_w, max_w);
                                self.crop_x0 = (self.crop_x1 - w).max(0.0);
                                self.crop_y0 = (self.crop_y1 - w * k).max(0.0);
                            }
                            CropDrag::TopRight => {
                                let w_raw = (self.crop_x1 - self.crop_x0 + dx).max(min_size);
                                let max_w =
                                    (1.0 - self.crop_x0).min(self.crop_y1 / k).max(0.01);
                                let min_w = min_size.min(max_w);
                                let w = w_raw.clamp(min_w, max_w);
                                self.crop_x1 = (self.crop_x0 + w).min(1.0);
                                self.crop_y0 = (self.crop_y1 - w * k).max(0.0);
                            }
                            CropDrag::BottomLeft => {
                                let w_raw = (self.crop_x1 - self.crop_x0 - dx).max(min_size);
                                let max_w =
                                    self.crop_x1.min((1.0 - self.crop_y0) / k).max(0.01);
                                let min_w = min_size.min(max_w);
                                let w = w_raw.clamp(min_w, max_w);
                                self.crop_x0 = (self.crop_x1 - w).max(0.0);
                                self.crop_y1 = (self.crop_y0 + w * k).min(1.0);
                            }
                            _ => {}
                        }
                    } else {
                        // Free aspect resize (safe clamp upper/lower bounds)
                        let max_x0 = (self.crop_x1 - min_size).max(0.0);
                        let min_x1 = (self.crop_x0 + min_size).min(1.0);
                        let max_y0 = (self.crop_y1 - min_size).max(0.0);
                        let min_y1 = (self.crop_y0 + min_size).min(1.0);
                        match drag {
                            CropDrag::TopLeft => {
                                self.crop_x0 = (self.crop_x0 + dx).clamp(0.0, max_x0);
                                self.crop_y0 = (self.crop_y0 + dy).clamp(0.0, max_y0);
                            }
                            CropDrag::TopRight => {
                                self.crop_x1 = (self.crop_x1 + dx).clamp(min_x1, 1.0);
                                self.crop_y0 = (self.crop_y0 + dy).clamp(0.0, max_y0);
                            }
                            CropDrag::BottomLeft => {
                                self.crop_x0 = (self.crop_x0 + dx).clamp(0.0, max_x0);
                                self.crop_y1 = (self.crop_y1 + dy).clamp(min_y1, 1.0);
                            }
                            CropDrag::BottomRight => {
                                self.crop_x1 = (self.crop_x1 + dx).clamp(min_x1, 1.0);
                                self.crop_y1 = (self.crop_y1 + dy).clamp(min_y1, 1.0);
                            }
                            CropDrag::Left => {
                                self.crop_x0 = (self.crop_x0 + dx).clamp(0.0, max_x0);
                            }
                            CropDrag::Right => {
                                self.crop_x1 = (self.crop_x1 + dx).clamp(min_x1, 1.0);
                            }
                            CropDrag::Top => {
                                self.crop_y0 = (self.crop_y0 + dy).clamp(0.0, max_y0);
                            }
                            CropDrag::Bottom => {
                                self.crop_y1 = (self.crop_y1 + dy).clamp(min_y1, 1.0);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if resp.drag_stopped() {
            self.active_drag = CropDrag::None;
            self.drag_aspect_ratio = None;
        }

        // Paint sleek framing overlay
        let crop_rect = Rect::from_min_max(
            Pos2::new(
                img_rect.left() + img_w * self.crop_x0,
                img_rect.top() + img_h * self.crop_y0,
            ),
            Pos2::new(
                img_rect.left() + img_w * self.crop_x1,
                img_rect.top() + img_h * self.crop_y1,
            ),
        );

        let dim = Color32::from_black_alpha(165);
        let p = ui.painter();

        // Dim outer areas
        p.rect_filled(
            Rect::from_min_max(img_rect.min, Pos2::new(img_rect.max.x, crop_rect.min.y)),
            0.0,
            dim,
        );
        p.rect_filled(
            Rect::from_min_max(Pos2::new(img_rect.min.x, crop_rect.max.y), img_rect.max),
            0.0,
            dim,
        );
        p.rect_filled(
            Rect::from_min_max(
                Pos2::new(img_rect.min.x, crop_rect.min.y),
                Pos2::new(crop_rect.min.x, crop_rect.max.y),
            ),
            0.0,
            dim,
        );
        p.rect_filled(
            Rect::from_min_max(
                Pos2::new(crop_rect.max.x, crop_rect.min.y),
                Pos2::new(img_rect.max.x, crop_rect.max.y),
            ),
            0.0,
            dim,
        );

        // Rule-of-thirds grid inside frame
        let grid_stroke = Stroke::new(1.0_f32, Color32::from_white_alpha(65));
        for i in 1..=2 {
            let frac = i as f32 / 3.0;
            let x = crop_rect.left() + crop_rect.width() * frac;
            let y = crop_rect.top() + crop_rect.height() * frac;
            p.line_segment(
                [Pos2::new(x, crop_rect.top()), Pos2::new(x, crop_rect.bottom())],
                grid_stroke,
            );
            p.line_segment(
                [Pos2::new(crop_rect.left(), y), Pos2::new(crop_rect.right(), y)],
                grid_stroke,
            );
        }

        // Outer frame border
        p.rect_stroke(crop_rect, 0.0, Stroke::new(1.5_f32, Color32::from_white_alpha(220)));

        // Crisp L-shaped corner handles
        let arm = 14.0_f32.min(crop_rect.width() * 0.25).min(crop_rect.height() * 0.25);
        let handle_stroke = Stroke::new(3.0_f32, Color32::WHITE);

        let tl = crop_rect.left_top();
        p.line_segment([tl, tl + Vec2::new(arm, 0.0)], handle_stroke);
        p.line_segment([tl, tl + Vec2::new(0.0, arm)], handle_stroke);

        let tr = crop_rect.right_top();
        p.line_segment([tr, tr + Vec2::new(-arm, 0.0)], handle_stroke);
        p.line_segment([tr, tr + Vec2::new(0.0, arm)], handle_stroke);

        let bl = crop_rect.left_bottom();
        p.line_segment([bl, bl + Vec2::new(arm, 0.0)], handle_stroke);
        p.line_segment([bl, bl + Vec2::new(0.0, -arm)], handle_stroke);

        let br = crop_rect.right_bottom();
        p.line_segment([br, br + Vec2::new(-arm, 0.0)], handle_stroke);
        p.line_segment([br, br + Vec2::new(0.0, -arm)], handle_stroke);
    }

    pub fn render_canvas(&mut self, ui: &mut egui::Ui) {
        self.rebuild_preview(ui.ctx());

        let (eff_w, eff_h) = self.effective_dimensions();

        // Top metadata bar
        let compact_bar = ui.available_width() < 420.0;
        let mut closed = false;
        ui.horizontal_wrapped(|ui| {
            if let Some(p) = &self.path {
                let full_name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Изображение");
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
                    .on_hover_text("Закрыть фото без удаления")
                    .clicked()
                {
                    self.clear();
                    closed = true;
                }
            }
            if !closed && self.crop_mode {
                ui.label(
                    egui::RichText::new(
                        "Тяни углы (Shift — пропорционально), клик мимо — применить",
                    )
                    .size(12.0)
                    .color(theme::ACCENT),
                );
            }
            if !closed && ui.max_rect().right() - ui.cursor().left() >= 165.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::metadata_pill(ui, &theme::format_bytes(self.file_size));
                    let out_dims = if self.custom_resize {
                        format!("{} × {}", self.target_width, self.target_height)
                    } else {
                        format!("{} × {}", eff_w, eff_h)
                    };
                    theme::metadata_pill(ui, &out_dims);
                });
            }
        });

        if closed || !self.has_file() {
            return;
        }

        ui.add_space(10.0);

        // Centered preview frame
        let avail = ui.available_size();
        egui::Frame::none()
            .fill(Color32::from_rgb(14, 14, 15))
            .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
            .rounding(Rounding::same(14.0))
            .show(ui, |ui| {
                ui.set_min_size(avail);
                let (rect, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
                let padded = rect.shrink(22.0);

                if let Some(tex) = &self.preview_texture {
                    let tex_size = tex.size_vec2();
                    if tex_size.x > 0.0 && tex_size.y > 0.0 {
                        let draw_size = if self.custom_resize
                            && !self.crop_mode
                            && self.target_width > 0
                            && self.target_height > 0
                        {
                            let (eff_w, eff_h) = self.effective_dimensions();
                            let base_fit = (padded.width() / eff_w.max(1) as f32)
                                .min(padded.height() / eff_h.max(1) as f32)
                                .min(1.0);
                            let desired = Vec2::new(
                                self.target_width as f32 * base_fit,
                                self.target_height as f32 * base_fit,
                            );
                            let cap = (padded.width() / desired.x)
                                .min(padded.height() / desired.y)
                                .min(1.0);
                            desired * cap
                        } else {
                            let scale = (padded.width() / tex_size.x)
                                .min(padded.height() / tex_size.y)
                                .min(1.0);
                            tex_size * scale
                        };
                        let img_rect = Rect::from_center_size(padded.center(), draw_size);

                        ui.painter().image(
                            tex.id(),
                            img_rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );

                        ui.painter().rect_stroke(
                            img_rect,
                            Rounding::same(2.0),
                            Stroke::new(1.0_f32, Color32::from_white_alpha(20)),
                        );

                        if self.crop_mode {
                            self.handle_crop_interaction(ui, rect, img_rect);
                        }
                    }
                }
            });
    }

    fn apply_width_with_aspect(&mut self, w: u32, aspect: f32) {
        let w = w.clamp(16, 8000);
        self.target_width = w;
        if self.lock_aspect {
            let aspect = aspect.max(0.01);
            let h_f = (w as f32 / aspect).round();
            if h_f > 8000.0 {
                self.target_height = 8000;
                self.target_width = ((8000.0 * aspect).round() as u32).clamp(16, 8000);
            } else if h_f < 16.0 {
                self.target_height = 16;
                self.target_width = ((16.0 * aspect).round() as u32).clamp(16, 8000);
            } else {
                self.target_height = h_f as u32;
            }
        }
    }

    fn apply_height_with_aspect(&mut self, h: u32, aspect: f32) {
        let h = h.clamp(16, 8000);
        self.target_height = h;
        if self.lock_aspect {
            let aspect = aspect.max(0.01);
            let w_f = (h as f32 * aspect).round();
            if w_f > 8000.0 {
                self.target_width = 8000;
                self.target_height = ((8000.0 / aspect).round() as u32).clamp(16, 8000);
            } else if w_f < 16.0 {
                self.target_width = 16;
                self.target_height = ((16.0 / aspect).round() as u32).clamp(16, 8000);
            } else {
                self.target_width = w_f as u32;
            }
        }
    }

    /// Renders right-side inspector controls. Returns `Some(PhotoExportJob)` when user chooses save path.
    pub fn render_inspector(&mut self, ui: &mut egui::Ui, is_busy: bool) -> Option<PhotoExportJob> {
        let mut job = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - 62.0)
            .show(ui, |ui| {
                // Framing / Crop card
                theme::section_card(ui, "КАДРИРОВАНИЕ", |ui| {
                    ui.horizontal(|ui| {
                        let btn_label = if self.crop_mode {
                            "✓  Применить кадр"
                        } else {
                            "✂  Кадрировать"
                        };
                        if theme::pill_choice(ui, self.crop_mode, btn_label).clicked() {
                            self.crop_mode = !self.crop_mode;
                            self.dirty = true;
                        }
                        if (self.is_cropped() || self.crop_mode)
                            && theme::subtle_button(ui, "Сбросить").clicked()
                        {
                            self.reset_crop();
                        }
                    });

                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Пропорции")
                            .size(11.5)
                            .color(theme::TEXT_MUTED),
                    );
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        for aspect in [
                            CropAspect::Free,
                            CropAspect::Square,
                            CropAspect::W16H9,
                            CropAspect::W4H3,
                            CropAspect::W9H16,
                        ] {
                            let selected = self.crop_mode && self.crop_aspect == aspect;
                            if theme::pill_choice(ui, selected, aspect.label()).clicked() {
                                self.apply_aspect_preset(aspect);
                            }
                        }
                    });
                });

                ui.add_space(10.0);

                // Transform card
                theme::section_card(ui, "ПОВОРОТ И ОТРАЖЕНИЕ", |ui| {
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.spacing_mut().button_padding = Vec2::new(6.0, 5.0);
                        let w = ((ui.available_width() - 6.0) / 2.0).floor();
                        ui.horizontal(|ui| {
                            if ui
                                .add_sized(
                                    Vec2::new(w, 30.0),
                                    egui::Button::new("↺  90° влево")
                                        .fill(theme::BG_ELEVATED)
                                        .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                        .rounding(Rounding::same(8.0)),
                                )
                                .clicked()
                            {
                                self.rotate_ccw();
                            }
                            if ui
                                .add_sized(
                                    Vec2::new(w, 30.0),
                                    egui::Button::new("↻  90° вправо")
                                        .fill(theme::BG_ELEVATED)
                                        .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                        .rounding(Rounding::same(8.0)),
                                )
                                .clicked()
                            {
                                self.rotate_cw();
                            }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add_sized(Vec2::new(w, 30.0), |ui: &mut egui::Ui| {
                                    theme::pill_choice(ui, self.flip_h, "⇄  Горизонтально")
                                })
                                .clicked()
                            {
                                self.flip_h = !self.flip_h;
                                self.dirty = true;
                            }
                            if ui
                                .add_sized(Vec2::new(w, 30.0), |ui: &mut egui::Ui| {
                                    theme::pill_choice(ui, self.flip_v, "⇅  Вертикально")
                                })
                                .clicked()
                            {
                                self.flip_v = !self.flip_v;
                                self.dirty = true;
                            }
                        });
                    });
                });

                ui.add_space(10.0);

                // Color & Light card
                theme::section_card(ui, "СВЕТ И ЦВЕТ", |ui| {
                    ui.scope(|ui| {
                        ui.spacing_mut().slider_width = (ui.available_width() - 66.0).max(110.0);
                        ui.spacing_mut().interact_size.x = 56.0;

                        ui.label(
                            egui::RichText::new("Яркость")
                                .size(12.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                        if ui
                            .add(egui::Slider::new(&mut self.brightness, -100..=100).show_value(true))
                            .changed()
                        {
                            self.dirty = true;
                        }

                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Контраст")
                                .size(12.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                        if ui
                            .add(
                                egui::Slider::new(&mut self.contrast, -80.0..=80.0)
                                    .fixed_decimals(0)
                                    .show_value(true),
                            )
                            .changed()
                        {
                            self.dirty = true;
                        }

                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Насыщенность")
                                .size(12.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                        let mut sat_pct = (self.saturation * 100.0).round();
                        if ui
                            .add(
                                egui::Slider::new(&mut sat_pct, 0.0..=200.0)
                                    .suffix("%")
                                    .fixed_decimals(0),
                            )
                            .changed()
                        {
                            self.saturation = sat_pct / 100.0;
                            self.dirty = true;
                        }
                    });

                    ui.add_space(6.0);
                    if ui.checkbox(&mut self.grayscale, "Чёрно-белый фильтр").changed() {
                        self.dirty = true;
                    }
                });

                ui.add_space(10.0);

                // Resize card
                theme::section_card(ui, "РАЗМЕР", |ui| {
                    let (eff_w, eff_h) = self.effective_dimensions();
                    if ui.checkbox(&mut self.custom_resize, "Изменить размер").changed() {
                        if self.custom_resize {
                            self.target_width = eff_w;
                            self.target_height = eff_h;
                            self.crop_mode = false;
                        }
                        self.dirty = true;
                    }
                    if self.custom_resize {
                        let aspect = (eff_w as f32) / (eff_h.max(1) as f32);
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Ш:").color(theme::TEXT_SECONDARY));
                            if ui
                                .add_sized(
                                    Vec2::new(68.0, 26.0),
                                    egui::DragValue::new(&mut self.target_width)
                                        .range(16..=8000)
                                        .speed(5),
                                )
                                .changed()
                            {
                                self.apply_width_with_aspect(self.target_width, aspect);
                                self.crop_mode = false;
                                self.dirty = true;
                            }
                            ui.label(egui::RichText::new("×").color(theme::TEXT_MUTED));
                            ui.label(egui::RichText::new("В:").color(theme::TEXT_SECONDARY));
                            if ui
                                .add_sized(
                                    Vec2::new(68.0, 26.0),
                                    egui::DragValue::new(&mut self.target_height)
                                        .range(16..=8000)
                                        .speed(5),
                                )
                                .changed()
                            {
                                self.apply_height_with_aspect(self.target_height, aspect);
                                self.crop_mode = false;
                                self.dirty = true;
                            }
                        });
                        ui.add_space(6.0);
                        ui.add_enabled_ui(self.lock_aspect, |ui| {
                            ui.spacing_mut().slider_width =
                                (ui.available_width() - 66.0).max(110.0);
                            ui.spacing_mut().interact_size.x = 56.0;
                            let mut scale_pct = ((self.target_width as f32 / eff_w.max(1) as f32)
                                * 100.0)
                                .round()
                                .clamp(10.0, 200.0);
                            ui.label(
                                egui::RichText::new("Масштаб")
                                    .size(12.0)
                                    .color(theme::TEXT_SECONDARY),
                            );
                            if ui
                                .add(
                                    egui::Slider::new(&mut scale_pct, 10.0..=200.0)
                                        .suffix("%")
                                        .fixed_decimals(0),
                                )
                                .on_disabled_hover_text("Включи «Сохранять пропорции»")
                                .changed()
                            {
                                let k = scale_pct / 100.0;
                                let raw_w = (eff_w as f32 * k).round() as u32;
                                self.apply_width_with_aspect(raw_w, aspect);
                                self.crop_mode = false;
                                self.dirty = true;
                            }
                        });
                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.lock_aspect, "Сохранять пропорции").changed()
                            && self.lock_aspect
                        {
                            self.apply_width_with_aspect(self.target_width, aspect);
                            self.dirty = true;
                        }
                    }
                });

                ui.add_space(10.0);

                // Export format card
                theme::section_card(ui, "ФОРМАТ СОХРАНЕНИЯ", |ui| {
                    ui.horizontal(|ui| {
                        for fmt in [PhotoFormat::Png, PhotoFormat::Jpeg, PhotoFormat::Webp] {
                            if theme::pill_choice(ui, self.format == fmt, fmt.label()).clicked() {
                                self.format = fmt;
                            }
                        }
                    });
                    if self.format == PhotoFormat::Jpeg {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Качество")
                                    .size(12.0)
                                    .color(theme::TEXT_SECONDARY),
                            );
                            let slider_w = (ui.available_width() - 62.0).max(70.0);
                            ui.scope(|ui| {
                                ui.spacing_mut().slider_width = slider_w;
                                ui.add(
                                    egui::Slider::new(&mut self.jpeg_quality, 50..=100).suffix("%"),
                                );
                            });
                        });
                    }
                });

                ui.add_space(8.0);
                if theme::subtle_button(ui, "Сбросить изменения").clicked() {
                    self.reset_edits();
                }
                ui.add_space(6.0);
            });

        if let Some(picked) = self.save_dialog.as_ref().and_then(|d| d.poll()) {
            self.save_dialog = None;
            if let (Some(orig), Some(output)) = (self.original.as_ref(), picked) {
                job = Some(PhotoExportJob {
                    original: Arc::clone(orig),
                    output,
                    rotation_steps: self.rotation_steps,
                    flip_h: self.flip_h,
                    flip_v: self.flip_v,
                    brightness: self.brightness,
                    contrast: self.contrast,
                    saturation: self.saturation,
                    grayscale: self.grayscale,
                    is_cropped: self.is_cropped(),
                    crop_x0: self.crop_x0,
                    crop_y0: self.crop_y0,
                    crop_x1: self.crop_x1,
                    crop_y1: self.crop_y1,
                    custom_resize: self.custom_resize,
                    target_width: self.target_width,
                    target_height: self.target_height,
                    format: self.format,
                    jpeg_quality: self.jpeg_quality,
                });
            }
        }

        ui.add_space(8.0);
        let btn_text = if is_busy {
            "Сохранение фото..."
        } else {
            "Сохранить фото"
        };
        ui.add_enabled_ui(!is_busy && self.save_dialog.is_none(), |ui| {
            if theme::accent_button(ui, btn_text).clicked() {
                self.request_save(ui.ctx());
            }
        });

        job
    }
}

fn apply_color_filters(
    img: DynamicImage,
    brightness: i32,
    contrast: f32,
    saturation: f32,
    grayscale: bool,
) -> DynamicImage {
    let has_brightness = brightness != 0;
    let has_contrast = contrast.abs() > 0.5;
    let has_sat = (saturation - 1.0).abs() > 0.01;
    if !has_brightness && !has_contrast && !grayscale && !has_sat {
        return img;
    }

    let mut rgba = img.into_rgba8();
    let c_pow = if has_contrast {
        ((100.0 + contrast) / 100.0).powi(2)
    } else {
        1.0
    };
    let b_add = brightness as f32;

    for p in rgba.pixels_mut() {
        let mut r = p[0] as f32;
        let mut g = p[1] as f32;
        let mut b = p[2] as f32;

        if has_brightness {
            r = (r + b_add).clamp(0.0, 255.0);
            g = (g + b_add).clamp(0.0, 255.0);
            b = (b + b_add).clamp(0.0, 255.0);
        }
        if has_contrast {
            r = (((r / 255.0 - 0.5) * c_pow + 0.5) * 255.0).clamp(0.0, 255.0);
            g = (((g / 255.0 - 0.5) * c_pow + 0.5) * 255.0).clamp(0.0, 255.0);
            b = (((b / 255.0 - 0.5) * c_pow + 0.5) * 255.0).clamp(0.0, 255.0);
        }
        if grayscale {
            let luma = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0);
            r = luma;
            g = luma;
            b = luma;
        } else if has_sat {
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            r = (luma + (r - luma) * saturation).clamp(0.0, 255.0);
            g = (luma + (g - luma) * saturation).clamp(0.0, 255.0);
            b = (luma + (b - luma) * saturation).clamp(0.0, 255.0);
        }

        p[0] = r as u8;
        p[1] = g as u8;
        p[2] = b as u8;
    }

    DynamicImage::ImageRgba8(rgba)
}

fn apply_photo_pipeline(
    src: &DynamicImage,
    params: &PhotoTransformParams,
    apply_crop: bool,
    apply_resize: bool,
    is_preview: bool,
) -> DynamicImage {
    let rot = params.rotation_steps % 4;
    let will_crop = apply_crop && params.is_cropped;

    // Avoid cloning full-res image when unrotated/unflipped and cropping or downscaling directly
    let mut img = if rot == 0 && !params.flip_h && !params.flip_v {
        if will_crop {
            let (w, h) = src.dimensions();
            let x = (((w as f32) * params.crop_x0).round() as u32).min(w.saturating_sub(1));
            let y = (((h as f32) * params.crop_y0).round() as u32).min(h.saturating_sub(1));
            let cw = (((w as f32) * (params.crop_x1 - params.crop_x0)).round() as u32)
                .min(w - x)
                .max(1);
            let ch = (((h as f32) * (params.crop_y1 - params.crop_y0)).round() as u32)
                .min(h - y)
                .max(1);
            src.crop_imm(x, y, cw, ch)
        } else {
            src.clone()
        }
    } else {
        let mut t = if rot == 0 {
            if params.flip_h {
                let f = src.fliph();
                if params.flip_v { f.flipv() } else { f }
            } else {
                src.flipv()
            }
        } else {
            let mut r = match rot {
                1 => src.rotate90(),
                2 => src.rotate180(),
                _ => src.rotate270(),
            };
            if params.flip_h {
                r = r.fliph();
            }
            if params.flip_v {
                r = r.flipv();
            }
            r
        };
        if will_crop {
            let (w, h) = t.dimensions();
            let x = (((w as f32) * params.crop_x0).round() as u32).min(w.saturating_sub(1));
            let y = (((h as f32) * params.crop_y0).round() as u32).min(h.saturating_sub(1));
            let cw = (((w as f32) * (params.crop_x1 - params.crop_x0)).round() as u32)
                .min(w - x)
                .max(1);
            let ch = (((h as f32) * (params.crop_y1 - params.crop_y0)).round() as u32)
                .min(h - y)
                .max(1);
            t = t.crop_imm(x, y, cw, ch);
        }
        t
    };

    let want_resize =
        apply_resize && params.custom_resize && params.target_width > 0 && params.target_height > 0;
    let resize_dims = if want_resize {
        if is_preview {
            if params.target_width <= 1920 && params.target_height <= 1920 {
                Some((params.target_width, params.target_height))
            } else {
                let k = (1920.0 / params.target_width as f32)
                    .min(1920.0 / params.target_height as f32);
                Some((
                    ((params.target_width as f32 * k).round() as u32).max(1),
                    ((params.target_height as f32 * k).round() as u32).max(1),
                ))
            }
        } else {
            Some((params.target_width, params.target_height))
        }
    } else {
        None
    };

    // Downscale BEFORE running per-pixel color adjustments for huge speedup on large photos
    let (cur_w, cur_h) = img.dimensions();
    let downscale_early = resize_dims
        .map(|(rw, rh)| (rw as u64 * rh as u64) < (cur_w as u64 * cur_h as u64))
        .unwrap_or(false);

    if downscale_early {
        if let Some((rw, rh)) = resize_dims {
            let filter = if is_preview {
                image::imageops::FilterType::Triangle
            } else {
                image::imageops::FilterType::Lanczos3
            };
            img = img.resize_exact(rw, rh, filter);
        }
    }

    img = apply_color_filters(
        img,
        params.brightness,
        params.contrast,
        params.saturation,
        params.grayscale,
    );

    if !downscale_early {
        if let Some((rw, rh)) = resize_dims {
            let filter = if is_preview {
                image::imageops::FilterType::Triangle
            } else {
                image::imageops::FilterType::Lanczos3
            };
            img = img.resize_exact(rw, rh, filter);
        }
    }

    img
}

pub fn run_photo_export(job: &PhotoExportJob) -> Result<PathBuf, String> {
    let params = PhotoTransformParams {
        rotation_steps: job.rotation_steps,
        flip_h: job.flip_h,
        flip_v: job.flip_v,
        brightness: job.brightness,
        contrast: job.contrast,
        saturation: job.saturation,
        grayscale: job.grayscale,
        is_cropped: job.is_cropped,
        crop_x0: job.crop_x0,
        crop_y0: job.crop_y0,
        crop_x1: job.crop_x1,
        crop_y1: job.crop_y1,
        custom_resize: job.custom_resize,
        target_width: job.target_width,
        target_height: job.target_height,
    };
    let processed = apply_photo_pipeline(&job.original, &params, true, true, false);

    let tmp_path = crate::media::temp_output_path(&job.output);
    let encode_res: Result<(), String> = (|| {
        match job.format {
            PhotoFormat::Png => {
                let file = std::fs::File::create(&tmp_path)
                    .map_err(|e| format!("Не удалось создать файл: {}", e))?;
                let writer = BufWriter::with_capacity(64 * 1024, file);
                let encoder = image::codecs::png::PngEncoder::new_with_quality(
                    writer,
                    image::codecs::png::CompressionType::Fast,
                    image::codecs::png::FilterType::Adaptive,
                );
                let rgba = processed.to_rgba8();
                encoder
                    .write_image(
                        rgba.as_raw(),
                        rgba.width(),
                        rgba.height(),
                        ExtendedColorType::Rgba8,
                    )
                    .map_err(|e| format!("Ошибка сохранения PNG: {}", e))
            }
            PhotoFormat::Jpeg => {
                let rgb = processed.to_rgb8();
                let file = std::fs::File::create(&tmp_path)
                    .map_err(|e| format!("Не удалось создать файл: {}", e))?;
                let mut writer = BufWriter::with_capacity(64 * 1024, file);
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    &mut writer,
                    job.jpeg_quality,
                );
                encoder
                    .encode_image(&rgb)
                    .map_err(|e| format!("Ошибка сохранения JPEG: {}", e))?;
                writer
                    .flush()
                    .map_err(|e| format!("Ошибка записи JPEG: {}", e))
            }
            PhotoFormat::Webp => processed
                .save_with_format(&tmp_path, ImageFormat::WebP)
                .map_err(|e| format!("Ошибка сохранения WEBP: {}", e)),
        }
    })();

    if let Err(e) = encode_res {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    crate::media::finalize_temp_replace(&tmp_path, &job.output)?;
    Ok(job.output.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn make_test_image(w: u32, h: u32) -> DynamicImage {
        let mut rgba = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let r = ((x * 37 + y * 13) % 256) as u8;
                let g = ((x * 17 + y * 53) % 256) as u8;
                let b = ((x * 29 + y * 7) % 256) as u8;
                rgba.put_pixel(x, y, Rgba([r, g, b, 255]));
            }
        }
        DynamicImage::ImageRgba8(rgba)
    }

    fn base_params() -> PhotoTransformParams {
        PhotoTransformParams {
            rotation_steps: 0,
            flip_h: false,
            flip_v: false,
            brightness: 0,
            contrast: 0.0,
            saturation: 1.0,
            grayscale: false,
            is_cropped: false,
            crop_x0: 0.0,
            crop_y0: 0.0,
            crop_x1: 1.0,
            crop_y1: 1.0,
            custom_resize: false,
            target_width: 0,
            target_height: 0,
        }
    }

    #[test]
    fn pipeline_handles_rotations_and_flips_without_panic() {
        let img = make_test_image(8, 6);

        let mut p = base_params();
        p.flip_h = true;
        let out_h = apply_photo_pipeline(&img, &p, true, true, false);
        assert_eq!(out_h.dimensions(), (8, 6));
        assert_eq!(
            out_h.to_rgba8().get_pixel(0, 0),
            img.to_rgba8().get_pixel(7, 0)
        );

        let mut p = base_params();
        p.flip_v = true;
        let out_v = apply_photo_pipeline(&img, &p, true, true, false);
        assert_eq!(out_v.dimensions(), (8, 6));
        assert_eq!(
            out_v.to_rgba8().get_pixel(0, 0),
            img.to_rgba8().get_pixel(0, 5)
        );

        let mut p = base_params();
        p.flip_h = true;
        p.flip_v = true;
        let out_hv = apply_photo_pipeline(&img, &p, true, true, false);
        assert_eq!(out_hv.dimensions(), (8, 6));
        assert_eq!(
            out_hv.to_rgba8().get_pixel(0, 0),
            img.to_rgba8().get_pixel(7, 5)
        );

        for rot in 1..=3 {
            let mut p = base_params();
            p.rotation_steps = rot;
            let out_r = apply_photo_pipeline(&img, &p, true, true, false);
            let expected = if rot % 2 == 1 { (6, 8) } else { (8, 6) };
            assert_eq!(out_r.dimensions(), expected);
        }
    }

    #[test]
    fn pipeline_crop_resize_and_color_filters() {
        let img = make_test_image(20, 12);
        let mut p = base_params();
        p.is_cropped = true;
        p.crop_x0 = 0.25;
        p.crop_y0 = 0.25;
        p.crop_x1 = 0.75;
        p.crop_y1 = 0.75;
        p.custom_resize = true;
        p.target_width = 16;
        p.target_height = 16;
        p.brightness = 10;
        p.contrast = 15.0;
        p.saturation = 1.2;
        p.grayscale = true;

        let out = apply_photo_pipeline(&img, &p, true, true, false);
        assert_eq!(out.dimensions(), (16, 16));
        let px = out.to_rgba8();
        let center = px.get_pixel(8, 8);
        assert_eq!(center[0], center[1]);
        assert_eq!(center[1], center[2]);
    }

    #[test]
    fn export_writes_png_jpeg_webp_and_overwrites_in_place() {
        let dir = std::env::temp_dir().join(format!("kadr_photo_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp test dir");

        let img = Arc::new(make_test_image(24, 18));
        let formats = [
            (PhotoFormat::Png, "out.png"),
            (PhotoFormat::Jpeg, "out.jpg"),
            (PhotoFormat::Webp, "out.webp"),
        ];

        for (fmt, name) in formats {
            let out_path = dir.join(name);
            let job = PhotoExportJob {
                original: Arc::clone(&img),
                output: out_path.clone(),
                rotation_steps: 0,
                flip_h: true,
                flip_v: false,
                brightness: 5,
                contrast: 0.0,
                saturation: 1.0,
                grayscale: false,
                is_cropped: false,
                crop_x0: 0.0,
                crop_y0: 0.0,
                crop_x1: 1.0,
                crop_y1: 1.0,
                custom_resize: false,
                target_width: 24,
                target_height: 18,
                format: fmt,
                jpeg_quality: 85,
            };

            let saved = run_photo_export(&job).expect("initial export should succeed");
            assert_eq!(saved, out_path);
            let first_len = std::fs::metadata(&out_path).unwrap().len();
            assert!(first_len > 0);
            let reopened = image::open(&out_path).expect("exported image should be readable");
            assert_eq!(reopened.dimensions(), (24, 18));

            // Test overwriting an existing file in place
            let saved_again = run_photo_export(&job).expect("overwrite export should succeed");
            assert_eq!(saved_again, out_path);
            assert!(std::fs::metadata(&out_path).unwrap().len() > 0);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
