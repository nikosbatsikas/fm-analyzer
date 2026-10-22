//! Custom instrument widgets: cards, segmented bar meters, LEDs, badges, LCD readouts.

use super::theme::*;
use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, FontId, Layout, Margin, Rect, RichText, Sense,
    Stroke, Ui,
};

/// A titled panel with a subtle border.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_BORDER))
        .rounding(6.0)
        .inner_margin(Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                section_title(ui, title);
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner
}

pub fn section_title(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title.to_uppercase()).size(10.5).strong().color(TEXT_DIM));
}

/// Colour of `value` according to the zone table `[(from, colour), ...]` (ascending).
pub fn zone_color(zones: &[(f32, Color32)], value: f32) -> Color32 {
    let mut c = zones.first().map(|z| z.1).unwrap_or(TEXT);
    for &(from, col) in zones {
        if value >= from {
            c = col;
        }
    }
    c
}

/// Horizontal segmented bar meter with coloured zones, scale and optional peak-hold marker.
pub struct Meter<'a> {
    pub label: &'a str,
    pub value: Option<f32>,
    pub unit: &'a str,
    pub decimals: usize,
    pub min: f32,
    pub max: f32,
    pub zones: &'a [(f32, Color32)],
    pub ticks: &'a [f32],
    pub hold: Option<f32>,
    pub signed: bool,
}

impl<'a> Meter<'a> {
    pub fn new(label: &'a str, unit: &'a str, min: f32, max: f32) -> Self {
        Self {
            label,
            value: None,
            unit,
            decimals: 1,
            min,
            max,
            zones: &[],
            ticks: &[],
            hold: None,
            signed: false,
        }
    }
    pub fn value(mut self, v: Option<f32>) -> Self {
        self.value = v;
        self
    }
    pub fn decimals(mut self, d: usize) -> Self {
        self.decimals = d;
        self
    }
    pub fn zones(mut self, z: &'a [(f32, Color32)]) -> Self {
        self.zones = z;
        self
    }
    pub fn ticks(mut self, t: &'a [f32]) -> Self {
        self.ticks = t;
        self
    }
    pub fn hold(mut self, h: Option<f32>) -> Self {
        self.hold = h;
        self
    }
    pub fn signed(mut self) -> Self {
        self.signed = true;
        self
    }

    pub fn show(self, ui: &mut Ui) {
        let width = ui.available_width();
        let col = self.value.map(|v| zone_color(self.zones, v)).unwrap_or(TEXT_FAINT);

        ui.horizontal(|ui| {
            ui.label(RichText::new(self.label).color(TEXT_DIM));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(self.unit).size(11.0).color(TEXT_DIM));
                let txt = match self.value {
                    Some(v) if self.signed => format!("{:+.*}", self.decimals, v),
                    Some(v) => format!("{:.*}", self.decimals, v),
                    None => "---".to_string(),
                };
                ui.label(RichText::new(txt).monospace().size(18.0).color(col));
            });
        });

        let (rect, _) = ui.allocate_exact_size(vec2(width, 12.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 2.0, INSET);
        let span = (self.max - self.min).max(1e-6);
        let x = |v: f32| rect.left() + ((v - self.min) / span).clamp(0.0, 1.0) * rect.width();
        let inner = rect.shrink(1.5);

        // dim zone backdrop so the scale is readable even when the bar is low
        for (i, &(from, c)) in self.zones.iter().enumerate() {
            let to = self.zones.get(i + 1).map(|z| z.0).unwrap_or(self.max);
            let r = Rect::from_min_max(pos2(x(from), inner.top()), pos2(x(to), inner.bottom()));
            if r.width() > 0.0 {
                p.rect_filled(r, 0.0, c.gamma_multiply(0.12));
            }
        }
        if let Some(v) = self.value {
            for (i, &(from, c)) in self.zones.iter().enumerate() {
                let to = self.zones.get(i + 1).map(|z| z.0).unwrap_or(self.max).min(v);
                let from = from.max(self.min);
                if to > from {
                    let r = Rect::from_min_max(pos2(x(from), inner.top()), pos2(x(to), inner.bottom()));
                    p.rect_filled(r, 0.0, c);
                }
            }
            if self.zones.is_empty() {
                let r = Rect::from_min_max(inner.left_top(), pos2(x(v), inner.bottom()));
                p.rect_filled(r, 0.0, ACCENT);
            }
        }
        // LED segment gaps
        let mut gx = rect.left() + 4.0;
        while gx < rect.right() - 1.0 {
            p.line_segment([pos2(gx, rect.top()), pos2(gx, rect.bottom())], Stroke::new(1.0, INSET));
            gx += 4.0;
        }
        if let Some(h) = self.hold {
            let hx = x(h);
            p.line_segment(
                [pos2(hx, rect.top() - 1.0), pos2(hx, rect.bottom() + 1.0)],
                Stroke::new(2.0, Color32::WHITE),
            );
        }

        if !self.ticks.is_empty() {
            let (trect, _) = ui.allocate_exact_size(vec2(width, 12.0), Sense::hover());
            let p = ui.painter();
            for (i, &t) in self.ticks.iter().enumerate() {
                let tx = x(t);
                p.line_segment([pos2(tx, trect.top()), pos2(tx, trect.top() + 3.0)], Stroke::new(1.0, TEXT_FAINT));
                let align = if i == 0 {
                    Align2::LEFT_TOP
                } else if i == self.ticks.len() - 1 {
                    Align2::RIGHT_TOP
                } else {
                    Align2::CENTER_TOP
                };
                let label = if self.signed && t > 0.0 { format!("+{t}") } else { format!("{t}") };
                p.text(pos2(tx, trect.top() + 2.0), align, label, FontId::proportional(9.5), TEXT_FAINT);
            }
        }
        ui.add_space(6.0);
    }
}

/// Status lamp with label.
pub fn led(ui: &mut Ui, label: &str, on: bool, color: Color32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
        let p = ui.painter();
        let c = rect.center();
        if on {
            p.circle_filled(c, 7.0, color.gamma_multiply(0.25));
            p.circle_filled(c, 4.5, color);
        } else {
            p.circle_filled(c, 4.5, CONTROL);
            p.circle_stroke(c, 4.5, Stroke::new(1.0, CARD_BORDER));
        }
        ui.label(RichText::new(label).size(11.5).strong().color(if on { TEXT } else { TEXT_FAINT }));
    });
}

/// Small pill label (e.g. TP / TA flags).
pub fn badge(ui: &mut Ui, text: &str, on: bool) {
    let (fill, fg) = if on { (ACCENT.gamma_multiply(0.25), ACCENT) } else { (INSET, TEXT_FAINT) };
    egui::Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0, if on { ACCENT.gamma_multiply(0.6) } else { CARD_BORDER }))
        .rounding(3.0)
        .inner_margin(Margin::symmetric(6.0, 1.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).strong().monospace().color(fg));
        });
}

/// Sunken LCD-style text box.
pub fn lcd(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    egui::Frame::none()
        .fill(INSET)
        .stroke(Stroke::new(1.0, CARD_BORDER))
        .rounding(4.0)
        .inner_margin(Margin::symmetric(10.0, 6.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).monospace().size(size).strong().color(color));
        });
}

/// Label / value row for property lists.
pub fn kv(ui: &mut Ui, key: &str, value: impl Into<RichText>) {
    ui.label(RichText::new(key).color(TEXT_DIM));
    ui.label(value);
    ui.end_row();
}

/// Filled button with a strong colour (primary actions).
pub fn primary_button(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
            .fill(color.gamma_multiply(0.8))
            .stroke(Stroke::new(1.0, color))
            .min_size(vec2(110.0, 28.0)),
    )
}
