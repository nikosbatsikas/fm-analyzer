//! Dark instrument theme: colour palette and egui style setup.

use eframe::egui::{self, vec2, Color32, FontFamily, FontId, Rounding, Stroke, TextStyle, Visuals};

// Surfaces, darkest to lightest
pub const BG: Color32 = Color32::from_rgb(0x0b, 0x0e, 0x12);
pub const PANEL: Color32 = Color32::from_rgb(0x11, 0x15, 0x1b);
pub const CARD: Color32 = Color32::from_rgb(0x17, 0x1c, 0x24);
pub const CARD_BORDER: Color32 = Color32::from_rgb(0x26, 0x2d, 0x38);
pub const INSET: Color32 = Color32::from_rgb(0x08, 0x0a, 0x0d);
pub const CONTROL: Color32 = Color32::from_rgb(0x20, 0x27, 0x31);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x29, 0x32, 0x3e);
pub const CONTROL_ACTIVE: Color32 = Color32::from_rgb(0x31, 0x3c, 0x4a);

// Text
pub const TEXT: Color32 = Color32::from_rgb(0xd6, 0xdc, 0xe4);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x7a, 0x85, 0x93);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x4a, 0x53, 0x5f);

// Signal colours
pub const ACCENT: Color32 = Color32::from_rgb(0x38, 0xb6, 0xff);
pub const LCD: Color32 = Color32::from_rgb(0x5c, 0xe1, 0xe6);
pub const GOOD: Color32 = Color32::from_rgb(0x34, 0xd3, 0x86);
pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xb0, 0x3a);
pub const BAD: Color32 = Color32::from_rgb(0xff, 0x52, 0x5a);

// Plot traces
pub const TRACE_RF: Color32 = Color32::from_rgb(0x38, 0xb6, 0xff);
pub const TRACE_MPX: Color32 = Color32::from_rgb(0x34, 0xd3, 0x86);
pub const TRACE_DEV: Color32 = Color32::from_rgb(0xf5, 0xb0, 0x3a);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.panel_fill = PANEL;
    v.window_fill = CARD;
    v.extreme_bg_color = INSET;
    v.faint_bg_color = Color32::from_rgb(0x1b, 0x21, 0x2a);
    v.code_bg_color = INSET;
    v.window_rounding = Rounding::same(6.0);
    v.menu_rounding = Rounding::same(6.0);
    v.window_stroke = Stroke::new(1.0, CARD_BORDER);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = BAD;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    let r = Rounding::same(4.0);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = CARD;
    w.noninteractive.weak_bg_fill = CARD;
    w.noninteractive.bg_stroke = Stroke::new(1.0, CARD_BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.noninteractive.rounding = r;

    w.inactive.bg_fill = CONTROL;
    w.inactive.weak_bg_fill = CONTROL;
    w.inactive.bg_stroke = Stroke::new(1.0, CARD_BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.rounding = r;

    w.hovered.bg_fill = CONTROL_HOVER;
    w.hovered.weak_bg_fill = CONTROL_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    w.hovered.rounding = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = CONTROL_ACTIVE;
    w.active.weak_bg_fill = CONTROL_ACTIVE;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, TEXT);
    w.active.rounding = r;
    w.active.expansion = 0.0;

    w.open = w.active;

    style.visuals = v;
    style.spacing.item_spacing = vec2(8.0, 6.0);
    style.spacing.button_padding = vec2(10.0, 4.0);
    style.spacing.interact_size.y = 24.0;
    style.spacing.slider_width = 140.0;

    style.text_styles = [
        (TextStyle::Heading, FontId::new(16.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(10.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();

    ctx.set_style(style);
}
