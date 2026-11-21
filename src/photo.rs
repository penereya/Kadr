use crate::theme;
use eframe::egui::{
    self, Color32, CursorIcon, Pos2, Rect, Rounding, Stroke, TextureHandle, TextureOptions, Vec2,
};
use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use std::path::{Path, PathBuf};

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
    original: Option<DynamicImage>,
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

        // Fast preview base (max 1000px on longest side)
        let preview = if w > 1000 || h > 1000 {
            img.thumbnail(1000, 1000)
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
        self.original = Some(img);
        self.preview_base = Some(preview);
        self.preview_texture = None;
        self.format = format;
        self.reset_edits();
        self.target_width = w;
        self.target_height = h;
        self.history.clear();
        self.frame_snapshot = None;
        Ok(())
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
        ctx.input(|i| {
            let no_mods = !i.modifiers.ctrl && !i.modifiers.command && !i.modifiers.alt;
            if no_mods && i.key_pressed(egui::Key::L) {
                self.rotation_steps = (self.rotation_steps + 3) % 4;
                self.dirty = true;
            }
            if no_mods && i.key_pressed(egui::Key::R) {
                self.rotation_steps = (self.rotation_steps + 1) % 4;
                self.dirty = true;
            }
            if no_mods && i.key_pressed(egui::Key::H) {
                self.flip_h = !self.flip_h;
                self.dirty = true;
            }
            if no_mods && i.key_pressed(egui::Key::V) {
                self.flip_v = !self.flip_v;
                self.dirty = true;
            }
            if no_mods && i.key_pressed(egui::Key::C) {
                self.crop_mode = !self.crop_mode;
                self.dirty = true;
            }
            if self.crop_mode && (i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape))
            {
                self.crop_mode = false;
                self.dirty = true;
            }
        });
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

    fn apply_transforms(
        &self,
        src: &DynamicImage,
        apply_crop: bool,
        apply_resize: bool,
        is_preview: bool,
    ) -> DynamicImage {
        let mut img = match self.rotation_steps % 4 {
            1 => src.rotate90(),
            2 => src.rotate180(),
            3 => src.rotate270(),
            _ => src.clone(),
        };

        if self.flip_h {
            img = img.fliph();
        }
        if self.flip_v {
            img = img.flipv();
        }

        if apply_crop && self.is_cropped() {
            let (w, h) = img.dimensions();
            let x = ((w as f32) * self.crop_x0).round() as u32;
            let y = ((h as f32) * self.crop_y0).round() as u32;
            let cw = ((w as f32) * (self.crop_x1 - self.crop_x0)).round() as u32;
            let ch = ((h as f32) * (self.crop_y1 - self.crop_y0)).round() as u32;
            let x = x.min(w.saturating_sub(1));
            let y = y.min(h.saturating_sub(1));
            let cw = cw.min(w - x).max(1);
            let ch = ch.min(h - y).max(1);
            img = img.crop_imm(x, y, cw, ch);
        }

        if self.brightness != 0 {
            img = img.brighten(self.brightness);
        }
        if self.contrast.abs() > 0.5 {
            img = img.adjust_contrast(self.contrast);
        }

        if self.grayscale {
            img = img.grayscale();
        } else if (self.saturation - 1.0).abs() > 0.01 {
            let mut rgba: RgbaImage = img.to_rgba8();
            let sat = self.saturation;
            for p in rgba.pixels_mut() {
                let r = p[0] as f32;
                let g = p[1] as f32;
                let b = p[2] as f32;
                let luma = 0.299 * r + 0.587 * g + 0.114 * b;
                p[0] = (luma + (r - luma) * sat).clamp(0.0, 255.0) as u8;
                p[1] = (luma + (g - luma) * sat).clamp(0.0, 255.0) as u8;
                p[2] = (luma + (b - luma) * sat).clamp(0.0, 255.0) as u8;
            }
            img = DynamicImage::ImageRgba8(rgba);
        }

        if apply_resize && self.custom_resize && self.target_width > 0 && self.target_height > 0 {
            if is_preview {
                let (pw, ph) = if self.target_width <= 1000 && self.target_height <= 1000 {
                    (self.target_width, self.target_height)
                } else {
                    let k = (1000.0 / self.target_width as f32)
                        .min(1000.0 / self.target_height as f32);
                    (
                        ((self.target_width as f32 * k).round() as u32).max(1),
                        ((self.target_height as f32 * k).round() as u32).max(1),
                    )
                };
                img = img.resize_exact(pw, ph, image::imageops::FilterType::Triangle);
            } else {
                img = img.resize_exact(
                    self.target_width,
                    self.target_height,
                    image::imageops::FilterType::Lanczos3,
                );
            }
        }

        img
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
        let processed = self.apply_transforms(base, !self.crop_mode, !self.crop_mode, true);
        let rgba = processed.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        self.preview_texture = Some(ctx.load_texture(
            "photo_preview",
            color_image,
            TextureOptions::LINEAR,
        ));
    }

    pub fn export(&self, out_path: &Path) -> Result<(), String> {
        let orig = self
            .original
            .as_ref()
            .ok_or_else(|| "Фото не загружено".to_string())?;
        let processed = self.apply_transforms(orig, true, true, false);

        match self.format {
            PhotoFormat::Png => processed
                .save_with_format(out_path, ImageFormat::Png)
                .map_err(|e| format!("Ошибка сохранения PNG: {}", e)),
            PhotoFormat::Jpeg => {
                let rgb = processed.to_rgb8();
                let mut file = std::fs::File::create(out_path)
                    .map_err(|e| format!("Не удалось создать файл: {}", e))?;
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    &mut file,
                    self.jpeg_quality,
                );
                encoder
                    .encode_image(&rgb)
                    .map_err(|e| format!("Ошибка сохранения JPEG: {}", e))
            }
            PhotoFormat::Webp => processed
                .save_with_format(out_path, ImageFormat::WebP)
                .map_err(|e| format!("Ошибка сохранения WEBP: {}", e)),
        }
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

    fn handle_crop_interaction(&mut self, ui: &mut egui::Ui, img_rect: Rect) {
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
            img_rect.expand(16.0),
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

        // Single click outside the crop frame automatically applies and closes framing mode
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if !crop_rect.expand(12.0).contains(pos) {
                    self.crop_mode = false;
                    self.dirty = true;
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
                    let w = self.crop_x1 - self.crop_x0;
                    let h = self.crop_y1 - self.crop_y0;
                    let nx0 = (self.crop_x0 + dx).clamp(0.0, 1.0 - w);
                    let ny0 = (self.crop_y0 + dy).clamp(0.0, 1.0 - h);
                    self.crop_x0 = nx0;
                    self.crop_x1 = nx0 + w;
                    self.crop_y0 = ny0;
                    self.crop_y1 = ny0 + h;
                }
                drag => {
                    let locked_ratio = self.crop_aspect.ratio().or(self.drag_aspect_ratio);
                    if let Some(target_ratio) = locked_ratio {
                        // Aspect-locked resize from corners or edges
                        // Normalized relation: (w * img_aspect) / h = target_ratio => h = w * (img_aspect / target_ratio)
                        let k = img_aspect / target_ratio;
                        match drag {
                            CropDrag::BottomRight | CropDrag::Right | CropDrag::Bottom => {
                                let mut w = (self.crop_x1 - self.crop_x0 + dx).max(min_size);
                                if drag == CropDrag::Bottom {
                                    let h = (self.crop_y1 - self.crop_y0 + dy).max(min_size);
                                    w = (h / k).max(min_size);
                                }
                                let max_w = (1.0 - self.crop_x0).min((1.0 - self.crop_y0) / k);
                                w = w.min(max_w).max(min_size);
                                self.crop_x1 = self.crop_x0 + w;
                                self.crop_y1 = self.crop_y0 + w * k;
                            }
                            CropDrag::TopLeft | CropDrag::Left | CropDrag::Top => {
                                let mut w = (self.crop_x1 - self.crop_x0 - dx).max(min_size);
                                if drag == CropDrag::Top {
                                    let h = (self.crop_y1 - self.crop_y0 - dy).max(min_size);
                                    w = (h / k).max(min_size);
                                }
                                let max_w = self.crop_x1.min(self.crop_y1 / k);
                                w = w.min(max_w).max(min_size);
                                self.crop_x0 = self.crop_x1 - w;
                                self.crop_y0 = self.crop_y1 - w * k;
                            }
                            CropDrag::TopRight => {
                                let mut w = (self.crop_x1 - self.crop_x0 + dx).max(min_size);
                                let max_w = (1.0 - self.crop_x0).min(self.crop_y1 / k);
                                w = w.min(max_w).max(min_size);
                                self.crop_x1 = self.crop_x0 + w;
                                self.crop_y0 = self.crop_y1 - w * k;
                            }
                            CropDrag::BottomLeft => {
                                let mut w = (self.crop_x1 - self.crop_x0 - dx).max(min_size);
                                let max_w = self.crop_x1.min((1.0 - self.crop_y0) / k);
                                w = w.min(max_w).max(min_size);
                                self.crop_x0 = self.crop_x1 - w;
                                self.crop_y1 = self.crop_y0 + w * k;
                            }
                            _ => {}
                        }
                    } else {
                        // Free aspect resize
                        match drag {
                            CropDrag::TopLeft => {
                                self.crop_x0 = (self.crop_x0 + dx).clamp(0.0, self.crop_x1 - min_size);
                                self.crop_y0 = (self.crop_y0 + dy).clamp(0.0, self.crop_y1 - min_size);
                            }
                            CropDrag::TopRight => {
                                self.crop_x1 = (self.crop_x1 + dx).clamp(self.crop_x0 + min_size, 1.0);
                                self.crop_y0 = (self.crop_y0 + dy).clamp(0.0, self.crop_y1 - min_size);
                            }
                            CropDrag::BottomLeft => {
                                self.crop_x0 = (self.crop_x0 + dx).clamp(0.0, self.crop_x1 - min_size);
                                self.crop_y1 = (self.crop_y1 + dy).clamp(self.crop_y0 + min_size, 1.0);
                            }
                            CropDrag::BottomRight => {
                                self.crop_x1 = (self.crop_x1 + dx).clamp(self.crop_x0 + min_size, 1.0);
                                self.crop_y1 = (self.crop_y1 + dy).clamp(self.crop_y0 + min_size, 1.0);
                            }
                            CropDrag::Left => {
                                self.crop_x0 = (self.crop_x0 + dx).clamp(0.0, self.crop_x1 - min_size);
                            }
                            CropDrag::Right => {
                                self.crop_x1 = (self.crop_x1 + dx).clamp(self.crop_x0 + min_size, 1.0);
                            }
                            CropDrag::Top => {
                                self.crop_y0 = (self.crop_y0 + dy).clamp(0.0, self.crop_y1 - min_size);
                            }
                            CropDrag::Bottom => {
                                self.crop_y1 = (self.crop_y1 + dy).clamp(self.crop_y0 + min_size, 1.0);
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
                    return;
                }
            }
            if self.crop_mode {
                ui.label(
                    egui::RichText::new(
                        "Тяни углы (Shift — пропорционально), клик мимо — применить",
                    )
                    .size(12.0)
                    .color(theme::ACCENT),
                );
            }
            if ui.max_rect().right() - ui.cursor().left() >= 165.0 {
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

        if !self.has_file() {
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
                            self.handle_crop_interaction(ui, img_rect);
                        }
                    }
                }
            });
    }

    /// Renders right-side inspector controls. Returns `Some(path)` if user requests saving.
    pub fn render_inspector(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let mut save_target = None;

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
                        if self.is_cropped() || self.crop_mode {
                            if theme::subtle_button(ui, "Сбросить").clicked() {
                                self.reset_crop();
                            }
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
                                self.rotation_steps = (self.rotation_steps + 3) % 4;
                                self.dirty = true;
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
                                self.rotation_steps = (self.rotation_steps + 1) % 4;
                                self.dirty = true;
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
                                if self.lock_aspect {
                                    self.target_height =
                                        ((self.target_width as f32) / aspect).round().max(1.0)
                                            as u32;
                                }
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
                                if self.lock_aspect {
                                    self.target_width =
                                        ((self.target_height as f32) * aspect).round().max(1.0)
                                            as u32;
                                }
                                self.crop_mode = false;
                                self.dirty = true;
                            }
                        });
                        ui.add_space(6.0);
                        ui.scope(|ui| {
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
                                .changed()
                            {
                                let k = scale_pct / 100.0;
                                self.target_width =
                                    ((eff_w as f32 * k).round() as u32).clamp(16, 8000);
                                self.target_height =
                                    ((eff_h as f32 * k).round() as u32).clamp(16, 8000);
                                self.crop_mode = false;
                                self.dirty = true;
                            }
                        });
                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.lock_aspect, "Сохранять пропорции").changed()
                            && self.lock_aspect
                        {
                            self.target_height =
                                ((self.target_width as f32) / aspect).round().max(1.0) as u32;
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
            if let Some(p) = picked {
                save_target = Some(p);
            }
        }

        ui.add_space(8.0);
        if theme::accent_button(ui, "Сохранить фото").clicked() {
            self.request_save(ui.ctx());
        }

        save_target
    }
}
