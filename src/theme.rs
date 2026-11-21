use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Margin, Rounding, Stroke, Vec2};

// Warm graphite + terracotta accent palette
pub const BG_BASE: Color32 = Color32::from_rgb(18, 19, 20);       // #121314
pub const BG_SURFACE: Color32 = Color32::from_rgb(26, 27, 29);    // #1A1B1D
pub const BG_ELEVATED: Color32 = Color32::from_rgb(36, 37, 40);   // #242528
pub const BG_HOVER: Color32 = Color32::from_rgb(47, 48, 52);      // #2F3034
pub const BORDER: Color32 = Color32::from_rgb(52, 53, 57);        // #343539
pub const BORDER_SUBTLE: Color32 = Color32::from_rgb(38, 39, 43); // #26272B

pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(240, 238, 235);   // #F0EEEB
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(160, 158, 154); // #A09E9A
pub const TEXT_MUTED: Color32 = Color32::from_rgb(108, 106, 102);     // #6C6A66

pub const ACCENT: Color32 = Color32::from_rgb(217, 83, 47);           // #D9532F
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(232, 101, 66);    // #E86542
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(54, 30, 24);       // #361E18

pub const SUCCESS: Color32 = Color32::from_rgb(74, 222, 128);
pub const ERROR: Color32 = Color32::from_rgb(248, 113, 113);

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    // Load clean Windows system font for crisp Cyrillic support
    let font_candidates = [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\tahoma.ttf",
    ];

    for path in font_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "system_ui".to_owned(),
                FontData::from_owned(bytes),
            );
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "system_ui".to_owned());
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push("system_ui".to_owned());
            break;
        }
    }

    ctx.set_fonts(fonts);
}

pub fn apply_graphite_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    visuals.dark_mode = true;
    visuals.panel_fill = BG_BASE;
    visuals.window_fill = BG_SURFACE;
    visuals.faint_bg_color = BG_ELEVATED;
    visuals.extreme_bg_color = Color32::from_rgb(14, 14, 15);
    visuals.code_bg_color = BG_ELEVATED;
    visuals.override_text_color = Some(TEXT_PRIMARY);

    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    visuals.window_rounding = Rounding::same(12.0);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.menu_rounding = Rounding::same(10.0);

    // Non-interactive widgets (labels, frames)
    visuals.widgets.noninteractive.bg_fill = BG_SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = BG_SURFACE;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER_SUBTLE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);
    visuals.widgets.noninteractive.rounding = Rounding::same(8.0);

    // Inactive interactive widgets
    visuals.widgets.inactive.bg_fill = BG_ELEVATED;
    visuals.widgets.inactive.weak_bg_fill = BG_ELEVATED;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);
    visuals.widgets.inactive.rounding = Rounding::same(8.0);

    // Hovered widgets
    visuals.widgets.hovered.bg_fill = BG_HOVER;
    visuals.widgets.hovered.weak_bg_fill = BG_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(76, 74, 72));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);
    visuals.widgets.hovered.rounding = Rounding::same(8.0);

    // Active widgets
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT_HOVER);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.active.rounding = Rounding::same(8.0);

    visuals.widgets.open.bg_fill = BG_HOVER;
    visuals.widgets.open.weak_bg_fill = BG_HOVER;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.open.rounding = Rounding::same(8.0);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 7.0);
    style.spacing.slider_width = 180.0;
    style.spacing.interact_size = Vec2::new(36.0, 28.0);
    style.interaction.tooltip_delay = 0.35;
    ctx.set_style(style);
}

pub fn section_card(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(BG_SURFACE)
        .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(title)
                    .size(12.0)
                    .color(TEXT_SECONDARY)
                    .strong(),
            );
            ui.add_space(8.0);
            add_contents(ui);
        });
}

pub fn accent_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let w = (ui.clip_rect().right() - ui.cursor().left() - 14.0)
        .min(ui.available_width())
        .max(80.0);
    let btn = egui::Button::new(
        egui::RichText::new(text)
            .size(13.5)
            .color(Color32::WHITE)
            .strong(),
    )
    .fill(ACCENT)
    .stroke(Stroke::NONE)
    .rounding(Rounding::same(9.0))
    .min_size(Vec2::new(w, 38.0));
    ui.add_sized(Vec2::new(w, 38.0), btn)
}

pub fn subtle_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let btn = egui::Button::new(egui::RichText::new(text).size(13.0).color(TEXT_PRIMARY))
        .fill(BG_ELEVATED)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(8.0));
    ui.add(btn)
}

pub fn danger_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let btn = egui::Button::new(
        egui::RichText::new(text)
            .size(13.0)
            .color(Color32::from_rgb(252, 165, 165)),
    )
    .fill(Color32::from_rgb(44, 24, 28))
    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(115, 52, 58)))
    .rounding(Rounding::same(8.0));
    ui.add(btn)
}

pub fn pill_choice(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let (fill, stroke, text_color) = if selected {
        (ACCENT_SOFT, Stroke::new(1.0_f32, ACCENT), TEXT_PRIMARY)
    } else {
        (BG_ELEVATED, Stroke::new(1.0_f32, BORDER_SUBTLE), TEXT_SECONDARY)
    };

    let btn = egui::Button::new(egui::RichText::new(label).size(12.5).color(text_color))
        .fill(fill)
        .stroke(stroke)
        .rounding(Rounding::same(7.0));
    ui.add(btn)
}

pub fn metadata_pill(ui: &mut egui::Ui, label: &str) {
    egui::Frame::none()
        .fill(BG_ELEVATED)
        .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
        .rounding(Rounding::same(6.0))
        .inner_margin(Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(label).size(11.5).color(TEXT_SECONDARY));
        });
}

#[derive(Clone, Copy)]
struct HoldRepeatState {
    started_at: std::time::Instant,
    last_tick: std::time::Instant,
    repeated: bool,
}

fn step_repeat_button(ui: &mut egui::Ui, id_salt: &str, label: &str, tooltip: &str) -> i32 {
    let resp = ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .size(13.0)
                .color(TEXT_PRIMARY)
                .strong(),
        )
        .fill(BG_ELEVATED)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(6.0))
        .min_size(Vec2::new(26.0, 24.0))
        .sense(egui::Sense::click_and_drag()),
    );

    let state_id = ui.make_persistent_id(id_salt);
    let now = std::time::Instant::now();
    let pointer_down = ui.ctx().input(|i| i.pointer.primary_down());
    let mut step = 0_i32;

    let existing = ui.ctx().data_mut(|d| d.get_temp::<HoldRepeatState>(state_id));

    if resp.is_pointer_button_down_on() || (existing.is_some() && pointer_down) {
        let mut st = existing.unwrap_or(HoldRepeatState {
            started_at: now,
            last_tick: now,
            repeated: false,
        });

        let held_secs = now.duration_since(st.started_at).as_secs_f32();
        if held_secs >= 0.25 && now.duration_since(st.last_tick).as_secs_f32() >= 0.03 {
            st.last_tick = now;
            st.repeated = true;
            step = if held_secs >= 1.5 {
                5
            } else if held_secs >= 0.6 {
                2
            } else {
                1
            };
        }

        ui.ctx().data_mut(|d| d.insert_temp(state_id, st));
        ui.ctx().request_repaint();
    } else {
        if let Some(st) = existing {
            if !st.repeated {
                step = 1;
            }
            ui.ctx()
                .data_mut(|d| d.remove::<HoldRepeatState>(state_id));
        } else if resp.clicked() {
            step = 1;
        }
        let _ = resp.on_hover_text(tooltip);
    }

    step
}

/// Volume slider with `-` / `+` step buttons that support single 1% clicks and accelerated press-and-hold.
pub fn volume_slider_with_steps(ui: &mut egui::Ui, volume_pct: &mut f32) -> bool {
    let mut changed = false;

    ui.horizontal(|ui| {
        let minus_step = step_repeat_button(
            ui,
            "vol_minus_hold",
            "−",
            "Убавить на 1% (можно зажать)",
        );
        if minus_step > 0 {
            *volume_pct = (volume_pct.round() - minus_step as f32).clamp(0.0, 300.0);
            changed = true;
        }

        let slider_w = (ui.available_width() - 36.0).max(90.0);
        let vol_resp = ui
            .scope(|ui| {
                ui.spacing_mut().slider_width = slider_w;
                ui.add(
                    egui::Slider::new(volume_pct, 0.0..=300.0)
                        .fixed_decimals(0)
                        .show_value(false),
                )
            })
            .inner;
        if vol_resp.changed() {
            *volume_pct = volume_pct.round();
            changed = true;
        }

        let plus_step = step_repeat_button(
            ui,
            "vol_plus_hold",
            "+",
            "Прибавить на 1% (можно зажать)",
        );
        if plus_step > 0 {
            *volume_pct = (volume_pct.round() + plus_step as f32).clamp(0.0, 300.0);
            changed = true;
        }
    });

    changed
}

pub fn format_time(secs: f64) -> String {
    let total = secs.max(0.0);
    let mins = (total / 60.0).floor() as u64;
    let rem = total - (mins as f64 * 60.0);
    format!("{:02}:{:04.1}", mins, rem)
}

pub fn parse_time(input: &str) -> Option<f64> {
    let clean = input.trim().replace(',', ".");
    if clean.is_empty() {
        return None;
    }
    let parts: Vec<&str> = clean.split(':').collect();
    match parts.len() {
        1 => parts[0]
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0),
        2 => {
            let mins = parts[0].trim().parse::<f64>().ok()?;
            let secs = parts[1].trim().parse::<f64>().ok()?;
            if mins >= 0.0 && secs >= 0.0 && mins.is_finite() && secs.is_finite() {
                Some(mins * 60.0 + secs)
            } else {
                None
            }
        }
        3 => {
            let hrs = parts[0].trim().parse::<f64>().ok()?;
            let mins = parts[1].trim().parse::<f64>().ok()?;
            let secs = parts[2].trim().parse::<f64>().ok()?;
            if hrs >= 0.0
                && mins >= 0.0
                && secs >= 0.0
                && hrs.is_finite()
                && mins.is_finite()
                && secs.is_finite()
            {
                Some(hrs * 3600.0 + mins * 60.0 + secs)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Editable time input (`MM:SS.d`, `HH:MM:SS`, or seconds). Returns `true` when user commits a new value.
pub fn editable_time_field(
    ui: &mut egui::Ui,
    id_salt: &str,
    secs: &mut f64,
    min_secs: f64,
    max_secs: f64,
) -> bool {
    let text_id = ui.make_persistent_id(id_salt);
    let has_focus_before = ui.memory(|m| m.has_focus(text_id));
    let mut buf = if has_focus_before {
        ui.ctx()
            .data_mut(|d| d.get_temp::<String>(text_id))
            .unwrap_or_else(|| format_time(*secs))
    } else {
        format_time(*secs)
    };

    let resp = ui
        .add(
            egui::TextEdit::singleline(&mut buf)
                .id(text_id)
                .desired_width(72.0)
                .horizontal_align(egui::Align::Center)
                .font(egui::FontId::proportional(12.0)),
        )
        .on_hover_text("Можно вписать время руками (например: 01:30 или 90) и нажать Enter");

    let mut committed = false;
    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));

    if resp.has_focus() {
        if esc {
            resp.surrender_focus();
            ui.ctx().data_mut(|d| d.remove::<String>(text_id));
        } else {
            ui.ctx().data_mut(|d| d.insert_temp(text_id, buf.clone()));
        }
    }

    if resp.lost_focus() {
        if !esc {
            if let Some(parsed) = parse_time(&buf) {
                let clamped = parsed.clamp(min_secs, max_secs.max(min_secs));
                if (clamped - *secs).abs() > 0.001 {
                    *secs = clamped;
                    committed = true;
                }
            }
        }
        ui.ctx().data_mut(|d| d.remove::<String>(text_id));
    }

    committed
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} ГБ", b / GB)
    } else if b >= MB {
        format!("{:.1} МБ", b / MB)
    } else if b >= KB {
        format!("{:.0} КБ", b / KB)
    } else {
        format!("{} Б", bytes)
    }
}
