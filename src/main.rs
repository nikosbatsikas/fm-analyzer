#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sound;
mod ui;

use eframe::egui::{self, vec2, Align, Color32, FontId, Key, Layout, Margin, RichText, Sense, Stroke};
use egui_plot::{HLine, Line, LineStyle, Plot, PlotPoints, VLine};
use fm_analyzer::audio::AudioOut;
use fm_analyzer::fm::{Snapshot, HISTORY};
use fm_analyzer::source::tcp::RtlTcpSource;
use fm_analyzer::source::{IqSource, R820T_GAINS};
use fm_analyzer::worker::{Cmd, Shared, Tuning, Worker};
use std::sync::Arc;
use std::time::Duration;
use ui::theme::*;
use ui::widgets::{badge, card, kv, lcd, led, primary_button, section_title, Meter};

const APP_NAME: &str = "FM Analyzer";
const DEFAULT_FREQ_MHZ: f64 = 98.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Usb,
    Tcp,
}

struct App {
    kind: SourceKind,
    host: String,
    port: u16,
    usb_index: u32,
    usb_devices: Vec<String>,
    freq_mhz: f64,
    freq_edit: Option<String>,
    freq_edit_focus: bool,
    gain_auto: bool,
    gain_idx: usize,
    ppm: i32,
    deemphasis_us: f32,
    force_mono: bool,
    volume: f32,
    muted: bool,
    shared: Arc<Shared>,
    audio: Arc<AudioOut>,
    sound: Option<sound::Sound>,
    sound_error: String,
    worker: Option<Worker>,
    snap: Option<Snapshot>,
    peak_hold: f32,
    error: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply(&cc.egui_ctx);
        let audio = Arc::new(AudioOut::new());
        let (sound, sound_error) = match sound::open(audio.clone()) {
            Ok(s) => (Some(s), String::new()),
            Err(e) => (None, format!("Audio: {e}")),
        };
        Self {
            kind: if cfg!(feature = "usb") { SourceKind::Usb } else { SourceKind::Tcp },
            host: "127.0.0.1".into(),
            port: 1234,
            usb_index: 0,
            usb_devices: Vec::new(),
            freq_mhz: DEFAULT_FREQ_MHZ,
            freq_edit: None,
            freq_edit_focus: false,
            gain_auto: true,
            gain_idx: 16,
            ppm: 0,
            deemphasis_us: 50.0,
            force_mono: false,
            volume: audio.volume(),
            muted: false,
            shared: Arc::new(Shared::default()),
            audio,
            sound,
            sound_error,
            worker: None,
            snap: None,
            peak_hold: 0.0,
            error: String::new(),
        }
    }

    fn gain(&self) -> Option<i32> {
        if self.gain_auto {
            None
        } else {
            Some((R820T_GAINS[self.gain_idx] * 10.0).round() as i32)
        }
    }

    fn freq_hz(&self) -> u32 {
        (self.freq_mhz * 1e6).round() as u32
    }

    fn open_source(&self) -> Result<Box<dyn IqSource>, String> {
        match self.kind {
            SourceKind::Tcp => RtlTcpSource::connect(&format!("{}:{}", self.host.trim(), self.port))
                .map(|s| Box::new(s) as Box<dyn IqSource>)
                .map_err(|e| format!("rtl_tcp: {e}")),
            #[cfg(feature = "usb")]
            SourceKind::Usb => fm_analyzer::source::usb::UsbSource::open(self.usb_index)
                .map(|s| Box::new(s) as Box<dyn IqSource>)
                .map_err(|e| e.to_string()),
            #[cfg(not(feature = "usb"))]
            SourceKind::Usb => Err("USB support was not compiled in (feature \"usb\")".into()),
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.stop();
        self.error.clear();
        self.peak_hold = 0.0;
        match self.open_source() {
            Ok(src) => {
                let ctx = ctx.clone();
                let tuning = Tuning {
                    freq_hz: self.freq_hz(),
                    gain: self.gain(),
                    ppm: self.ppm,
                    deemphasis_us: self.deemphasis_us,
                    force_mono: self.force_mono,
                };
                self.worker = Some(Worker::spawn(
                    src,
                    tuning,
                    self.shared.clone(),
                    self.audio.clone(),
                    Box::new(move || ctx.request_repaint()),
                ));
            }
            Err(e) => self.error = e,
        }
    }

    fn stop(&mut self) {
        if let Some(w) = self.worker.take() {
            w.stop();
        }
        self.audio.clear();
    }

    fn send(&self, cmd: Cmd) {
        if let Some(w) = &self.worker {
            w.send(cmd);
        }
    }

    fn retune(&mut self) {
        self.freq_mhz = (self.freq_mhz.clamp(64.0, 108.0) * 1000.0).round() / 1000.0;
        self.peak_hold = 0.0;
        self.send(Cmd::SetFreq(self.freq_hz()));
    }

    fn step_freq(&mut self, mhz: f64) {
        self.freq_mhz += mhz;
        self.retune();
    }

    fn running(&self) -> bool {
        *self.shared.running.lock().unwrap()
    }

    // ------------------------------------------------------------------ header

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("FM").size(20.0).strong().color(ACCENT));
            ui.label(RichText::new("ANALYZER").size(20.0).strong().color(TEXT));
            ui.label(
                RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                    .size(11.0)
                    .color(TEXT_FAINT),
            );
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            let connected = self.worker.is_some();
            ui.add_enabled_ui(!connected, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    #[cfg(feature = "usb")]
                    ui.selectable_value(&mut self.kind, SourceKind::Usb, " USB ");
                    ui.selectable_value(&mut self.kind, SourceKind::Tcp, " RTL-TCP ");
                });
                ui.add_space(6.0);
                match self.kind {
                    SourceKind::Tcp => {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.host)
                                .desired_width(130.0)
                                .hint_text("host"),
                        );
                        ui.label(RichText::new(":").color(TEXT_DIM));
                        ui.add(egui::DragValue::new(&mut self.port));
                    }
                    SourceKind::Usb => {
                        ui.label(RichText::new("Device").color(TEXT_DIM));
                        ui.add(egui::DragValue::new(&mut self.usb_index).prefix("#"));
                        #[cfg(feature = "usb")]
                        if ui.button("Scan").clicked() {
                            match fm_analyzer::source::usb::list_devices() {
                                Ok(list) if list.is_empty() => self.error = "No RTL-SDR found".into(),
                                Ok(list) => {
                                    self.usb_devices = list;
                                    self.error.clear();
                                }
                                Err(e) => self.error = e.to_string(),
                            }
                        }
                        if let Some(name) = self.usb_devices.get(self.usb_index as usize) {
                            ui.label(RichText::new(name).size(11.5).color(TEXT_DIM));
                        }
                    }
                }
            });
            ui.add_space(8.0);
            let ctx = ui.ctx().clone();
            if connected {
                if primary_button(ui, "DISCONNECT", BAD).clicked() {
                    self.stop();
                }
            } else if primary_button(ui, "CONNECT", GOOD).clicked() {
                self.start(&ctx);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let s = self.snap.as_ref().filter(|_| self.worker.is_some());
                let over = s.map(|s| s.peak_khz > 75.0).unwrap_or(false);
                let rds = s.map(|s| s.rds.synced).unwrap_or(false);
                let stereo = s.map(|s| s.stereo).unwrap_or(false);
                let carrier = s.map(|s| s.channel_dbfs > -45.0).unwrap_or(false);
                led(ui, "OVER", over, BAD);
                ui.add_space(6.0);
                led(ui, "RDS", rds, ACCENT);
                ui.add_space(6.0);
                led(ui, "STEREO", stereo, GOOD);
                ui.add_space(6.0);
                led(ui, "SIGNAL", carrier, GOOD);
            });
        });
    }

    // ------------------------------------------------------------- tuner bar

    fn tuner_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // frequency LCD
            let mut action: Option<f64> = None; // new frequency
            let mut close_edit = false;
            if let Some(text) = &mut self.freq_edit {
                let r = ui.add(
                    egui::TextEdit::singleline(text)
                        .font(FontId::monospace(30.0))
                        .text_color(LCD)
                        .desired_width(190.0),
                );
                if self.freq_edit_focus {
                    r.request_focus();
                    self.freq_edit_focus = false;
                }
                if r.lost_focus() {
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        if let Ok(v) = text.trim().replace(',', ".").parse::<f64>() {
                            // allow "985" or "98500" style input
                            let v = if v > 10_000.0 { v / 1000.0 } else if v > 200.0 { v / 10.0 } else { v };
                            action = Some(v);
                        }
                    }
                    close_edit = true;
                }
            } else {
                let resp = egui::Frame::none()
                    .fill(INSET)
                    .stroke(Stroke::new(1.0, CARD_BORDER))
                    .rounding(4.0)
                    .inner_margin(Margin::symmetric(14.0, 2.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{:7.3}", self.freq_mhz))
                                    .font(FontId::monospace(30.0))
                                    .color(LCD),
                            );
                            ui.label(RichText::new("MHz").size(12.0).color(TEXT_DIM));
                        });
                    })
                    .response
                    .interact(Sense::click())
                    .on_hover_text("Click to type a frequency, scroll to tune (100 kHz)");
                if resp.clicked() {
                    self.freq_edit = Some(format!("{:.3}", self.freq_mhz));
                    self.freq_edit_focus = true;
                }
                if resp.hovered() {
                    let dy = ui.input(|i| i.raw_scroll_delta.y);
                    if dy > 0.0 {
                        action = Some(self.freq_mhz + 0.1);
                    } else if dy < 0.0 {
                        action = Some(self.freq_mhz - 0.1);
                    }
                }
            }
            if close_edit {
                self.freq_edit = None;
            }
            if let Some(f) = action {
                self.freq_mhz = f;
                self.retune();
            }

            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                ui.horizontal(|ui| {
                    if ui.small_button(" -1M ").clicked() {
                        self.step_freq(-1.0);
                    }
                    if ui.small_button("-100k").clicked() {
                        self.step_freq(-0.1);
                    }
                    if ui.small_button("+100k").clicked() {
                        self.step_freq(0.1);
                    }
                    if ui.small_button(" +1M ").clicked() {
                        self.step_freq(1.0);
                    }
                });
                ui.horizontal(|ui| {
                    if ui.small_button(" -50k ").clicked() {
                        self.step_freq(-0.05);
                    }
                    if ui.small_button(" -10k ").clicked() {
                        self.step_freq(-0.01);
                    }
                    if ui.small_button(" +10k ").clicked() {
                        self.step_freq(0.01);
                    }
                    if ui.small_button(" +50k ").clicked() {
                        self.step_freq(0.05);
                    }
                });
            });

            ui.add_space(10.0);
            ui.separator();

            // RF
            ui.vertical(|ui| {
                section_title(ui, "RF gain");
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut self.gain_auto, "Auto").changed() {
                        self.send(Cmd::SetGain(self.gain()));
                    }
                    ui.add_enabled_ui(!self.gain_auto, |ui| {
                        let max = R820T_GAINS.len() - 1;
                        let label = format!("{:4.1} dB", R820T_GAINS[self.gain_idx]);
                        let r = ui.add(
                            egui::Slider::new(&mut self.gain_idx, 0..=max)
                                .show_value(false)
                                .text(label),
                        );
                        if r.changed() {
                            self.send(Cmd::SetGain(self.gain()));
                        }
                    });
                });
            });
            ui.vertical(|ui| {
                section_title(ui, "PPM");
                if ui.add(egui::DragValue::new(&mut self.ppm).speed(0.2)).changed() {
                    self.ppm = self.ppm.clamp(-200, 200);
                    self.send(Cmd::SetPpm(self.ppm));
                }
            });

            ui.separator();

            // Audio
            ui.vertical(|ui| {
                section_title(ui, "Audio");
                ui.horizontal(|ui| {
                    let label = if self.muted { "MUTED" } else { "ON" };
                    if ui
                        .add(egui::SelectableLabel::new(!self.muted, RichText::new(label).strong()))
                        .clicked()
                    {
                        self.muted = !self.muted;
                        self.audio.set_muted(self.muted);
                    }
                    let label = format!("{:3.0} %", self.volume * 100.0);
                    let r = ui.add(
                        egui::Slider::new(&mut self.volume, 0.0..=1.5)
                            .show_value(false)
                            .text(label),
                    );
                    if r.changed() {
                        self.audio.set_volume(self.volume);
                    }
                });
            });
            ui.vertical(|ui| {
                section_title(ui, "Mode");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if ui.selectable_label(!self.force_mono, " Stereo ").clicked() && self.force_mono {
                        self.force_mono = false;
                        self.send(Cmd::SetMono(false));
                    }
                    if ui.selectable_label(self.force_mono, " Mono ").clicked() && !self.force_mono {
                        self.force_mono = true;
                        self.send(Cmd::SetMono(true));
                    }
                });
            });
            ui.vertical(|ui| {
                section_title(ui, "De-emphasis");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (us, name) in [(50.0, " 50 µs "), (75.0, " 75 µs "), (0.0, " Off ")] {
                        if ui.selectable_label(self.deemphasis_us == us, name).clicked() {
                            self.deemphasis_us = us;
                            self.send(Cmd::SetDeemphasis(us));
                        }
                    }
                });
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Reset").on_hover_text("Clear RDS, peak hold and MPX history").clicked() {
                    self.peak_hold = 0.0;
                    self.send(Cmd::ResetRds);
                }
            });
        });
    }

    // ---------------------------------------------------------------- meters

    fn meters(&mut self, ui: &mut egui::Ui) {
        let s = self.snap.as_ref();
        let v = |f: fn(&Snapshot) -> f32| s.map(f);

        card(ui, "Signal", |ui| {
            Meter::new("Channel level", "dBFS", -60.0, 0.0)
                .value(v(|s| s.channel_dbfs))
                .zones(&[(-60.0, WARN), (-45.0, GOOD), (-6.0, BAD)])
                .ticks(&[-60.0, -45.0, -30.0, -15.0, 0.0])
                .show(ui);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Carrier offset").color(TEXT_DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let t = s.map(|s| format!("{:+.2} kHz", s.offset_khz)).unwrap_or("---".into());
                    ui.label(RichText::new(t).monospace());
                });
            });
        });
        ui.add_space(8.0);

        card(ui, "Modulation", |ui| {
            Meter::new("Peak deviation", "kHz", 0.0, 120.0)
                .value(v(|s| s.peak_khz))
                .hold(s.map(|_| self.peak_hold))
                .zones(&[(0.0, GOOD), (70.0, WARN), (75.0, BAD)])
                .ticks(&[0.0, 25.0, 50.0, 75.0, 100.0, 120.0])
                .show(ui);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Peak hold").color(TEXT_DIM));
                ui.label(
                    RichText::new(format!("{:.1} kHz", self.peak_hold))
                        .monospace()
                        .color(if self.peak_hold > 75.0 { BAD } else { TEXT }),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let t = s.map(|s| format!("{:.2} %", s.over75_pct)).unwrap_or("---".into());
                    let c = if s.map(|s| s.over75_pct > 0.0).unwrap_or(false) { WARN } else { TEXT };
                    ui.label(RichText::new(t).monospace().color(c));
                    ui.label(RichText::new("> 75k").color(TEXT_DIM));
                });
            });
            ui.add_space(4.0);
            Meter::new("MPX power (60 s)", "dBr", -12.0, 6.0)
                .value(v(|s| s.mpx_power_60s))
                .decimals(2)
                .signed()
                .zones(&[(-12.0, GOOD), (0.0, BAD)])
                .ticks(&[-12.0, -9.0, -6.0, -3.0, 0.0, 3.0, 6.0])
                .show(ui);
            Meter::new("MPX power (100 ms)", "dBr", -12.0, 6.0)
                .value(v(|s| s.mpx_power_inst))
                .decimals(2)
                .signed()
                .zones(&[(-12.0, GOOD), (0.0, BAD)])
                .show(ui);
        });
        ui.add_space(8.0);

        card(ui, "Pilot & RDS", |ui| {
            let locked = s.map(|s| s.pilot_locked).unwrap_or(false);
            Meter::new("Pilot 19 kHz", "kHz", 0.0, 10.0)
                .value(s.filter(|_| locked).map(|s| s.pilot_khz))
                .decimals(2)
                .zones(&[(0.0, WARN), (6.0, GOOD), (7.5, WARN)])
                .ticks(&[0.0, 2.0, 4.0, 6.0, 7.5, 10.0])
                .show(ui);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Pilot frequency").color(TEXT_DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let t = s
                        .filter(|_| locked)
                        .map(|s| format!("{:.2} Hz", s.pilot_freq_hz))
                        .unwrap_or("mono".into());
                    ui.label(RichText::new(t).monospace());
                });
            });
            ui.add_space(4.0);
            let rds = s.map(|s| s.rds.synced).unwrap_or(false);
            Meter::new("RDS 57 kHz (approx.)", "kHz", 0.0, 10.0)
                .value(s.filter(|_| rds).map(|s| s.rds_khz))
                .decimals(2)
                .zones(&[(0.0, WARN), (2.0, GOOD), (7.5, WARN)])
                .ticks(&[0.0, 2.0, 4.0, 6.0, 8.0, 10.0])
                .show(ui);
        });
        ui.add_space(8.0);

        card(ui, "Audio output", |ui| {
            let zones = [(-60.0, GOOD), (-6.0, WARN), (-1.0, BAD)];
            let ticks = [-48.0, -36.0, -24.0, -12.0, -6.0, 0.0];
            Meter::new("Left", "dBFS", -48.0, 0.0)
                .value(v(|s| s.audio_l_db))
                .zones(&zones)
                .show(ui);
            Meter::new("Right", "dBFS", -48.0, 0.0)
                .value(v(|s| s.audio_r_db))
                .zones(&zones)
                .ticks(&ticks)
                .show(ui);
            let mode = match s {
                Some(s) if s.stereo => ("STEREO", GOOD),
                Some(_) => ("MONO", TEXT_DIM),
                None => ("---", TEXT_FAINT),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(mode.0).strong().color(mode.1));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let t = match &self.sound {
                        Some(snd) => format!("{} Hz · {:.0} ms", snd.rate, self.audio.fill_secs() * 1000.0),
                        None => "no output".into(),
                    };
                    ui.label(RichText::new(t).size(11.0).color(TEXT_DIM));
                });
            });
        });
    }

    // ------------------------------------------------------------------- RDS

    fn rds_panel(&self, ui: &mut egui::Ui) {
        let Some(s) = &self.snap else {
            card(ui, "RDS", |ui| {
                ui.label(RichText::new("No data").color(TEXT_FAINT));
            });
            return;
        };
        let r = &s.rds;
        card(ui, "RDS", |ui| {
            lcd(ui, &r.ps_string(), 30.0, if r.synced { LCD } else { TEXT_FAINT });
            ui.add_space(8.0);
            egui::Grid::new("rds_grid").num_columns(2).spacing([14.0, 5.0]).show(ui, |ui| {
                kv(
                    ui,
                    "PI",
                    RichText::new(r.pi.map(|p| format!("{p:04X}")).unwrap_or("----".into()))
                        .monospace()
                        .size(16.0)
                        .strong()
                        .color(TEXT),
                );
                kv(
                    ui,
                    "PTY",
                    RichText::new(format!(
                        "{}  {}",
                        r.pty.map(|p| format!("{p:2}")).unwrap_or("--".into()),
                        r.pty_name()
                    ))
                    .monospace(),
                );
                ui.label(RichText::new("Flags").color(TEXT_DIM));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    badge(ui, "TP", r.tp);
                    badge(ui, "TA", r.ta);
                    badge(ui, if r.music { "MUSIC" } else { "SPEECH" }, r.pty.is_some());
                });
                ui.end_row();
                kv(ui, "Clock", RichText::new(r.clock.clone().unwrap_or("-".into())).monospace().size(12.0));
                let bler_c = if r.bler() > 10.0 { BAD } else if r.bler() > 2.0 { WARN } else { GOOD };
                kv(
                    ui,
                    "BLER",
                    RichText::new(format!("{:.1} %", r.bler())).monospace().color(if r.blocks_total > 0 {
                        bler_c
                    } else {
                        TEXT_FAINT
                    }),
                );
                kv(ui, "Groups", RichText::new(r.total_groups().to_string()).monospace());
            });
        });
        ui.add_space(8.0);
        card(ui, "Radiotext", |ui| {
            let rt = r.rt_string();
            egui::Frame::none()
                .fill(INSET)
                .rounding(4.0)
                .inner_margin(Margin::same(8.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(36.0);
                    ui.label(
                        RichText::new(if rt.is_empty() { "—".to_string() } else { rt })
                            .monospace()
                            .color(if r.synced { TEXT } else { TEXT_FAINT }),
                    );
                });
        });
        ui.add_space(8.0);
        card(ui, "Alternative frequencies", |ui| {
            if r.af.is_empty() {
                ui.label(RichText::new("none received").color(TEXT_FAINT));
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for f in &r.af {
                        badge(ui, &format!("{:.1}", *f as f32 / 10.0), true);
                    }
                });
            }
        });
        ui.add_space(8.0);
        card(ui, "Group statistics", |ui| {
            egui::Grid::new("groups").striped(true).spacing([18.0, 3.0]).show(ui, |ui| {
                let mut any = false;
                for (t, g) in r.groups.iter().enumerate() {
                    for (ver, n) in g.iter().enumerate() {
                        if *n > 0 {
                            any = true;
                            ui.label(
                                RichText::new(format!("{t}{}", if ver == 0 { 'A' } else { 'B' }))
                                    .monospace()
                                    .strong(),
                            );
                            ui.label(RichText::new(n.to_string()).monospace());
                            ui.end_row();
                        }
                    }
                }
                if !any {
                    ui.label(RichText::new("-").color(TEXT_FAINT));
                    ui.end_row();
                }
            });
        });
    }

    // ----------------------------------------------------------------- plots

    fn plots(&self, ui: &mut egui::Ui) {
        let overhead = 3.0 * 50.0 + 2.0 * 8.0;
        let h = ((ui.available_height() - overhead) / 3.0).max(110.0);
        let center = self.freq_mhz;
        let s = self.snap.as_ref();
        let grid = Color32::from_rgba_unmultiplied(255, 255, 255, 18);

        card(ui, "RF spectrum · dBFS", |ui| {
            let pts: Vec<[f64; 2]> = s
                .map(|s| {
                    let n = s.rf_db.len();
                    let bin = s.rf_bin_hz() as f64;
                    s.rf_db
                        .iter()
                        .enumerate()
                        .map(|(k, &db)| {
                            [center + (k as f64 - n as f64 / 2.0) * bin / 1e6, db.clamp(-110.0, 0.0) as f64]
                        })
                        .collect()
                })
                .unwrap_or_default();
            Plot::new("rf")
                .height(h)
                .show_background(false)
                .include_y(-110.0)
                .include_y(-10.0)
                .include_x(center - 1.14)
                .include_x(center + 1.14)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .allow_boxed_zoom(false)
                .x_axis_label("MHz")
                .show(ui, |p| {
                    p.vline(VLine::new(center - 0.1).color(grid).style(LineStyle::dashed_loose()));
                    p.vline(VLine::new(center + 0.1).color(grid).style(LineStyle::dashed_loose()));
                    p.vline(VLine::new(center).color(LCD.gamma_multiply(0.5)));
                    p.line(Line::new(PlotPoints::new(pts)).color(TRACE_RF).width(1.2).fill(-110.0));
                });
        });
        ui.add_space(8.0);

        card(ui, "MPX spectrum · dB re 1 kHz deviation", |ui| {
            let pts: Vec<[f64; 2]> = s
                .map(|s| {
                    let bin = s.mpx_bin_hz() as f64;
                    s.mpx_db
                        .iter()
                        .enumerate()
                        .map(|(k, &db)| [k as f64 * bin / 1000.0, db.clamp(-60.0, 40.0) as f64])
                        .collect()
                })
                .unwrap_or_default();
            Plot::new("mpx")
                .height(h)
                .show_background(false)
                .include_y(-60.0)
                .include_y(40.0)
                .include_x(0.0)
                .include_x(100.0)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .allow_boxed_zoom(false)
                .x_axis_label("kHz")
                .show(ui, |p| {
                    for (f, name) in [
                        (15.0, "15 kHz"),
                        (19.0, "Pilot"),
                        (23.0, "23 kHz"),
                        (38.0, "38 kHz"),
                        (53.0, "53 kHz"),
                        (57.0, "RDS"),
                    ] {
                        p.vline(
                            VLine::new(f)
                                .name(name)
                                .color(WARN.gamma_multiply(0.35))
                                .style(LineStyle::dashed_loose()),
                        );
                    }
                    p.line(Line::new(PlotPoints::new(pts)).color(TRACE_MPX).width(1.2).fill(-60.0));
                });
        });
        ui.add_space(8.0);

        card(ui, "Peak deviation · last 60 s · kHz", |ui| {
            let pts: Vec<[f64; 2]> = s
                .map(|s| {
                    let len = s.dev_history.len();
                    s.dev_history
                        .iter()
                        .enumerate()
                        .map(|(k, &v)| [(k as f64 - len as f64) / 10.0, v.clamp(0.0, 150.0) as f64])
                        .collect()
                })
                .unwrap_or_default();
            Plot::new("dev")
                .height(h)
                .show_background(false)
                .include_y(0.0)
                .include_y(100.0)
                .include_x(-(HISTORY as f64) / 10.0)
                .include_x(0.0)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .allow_boxed_zoom(false)
                .x_axis_label("seconds")
                .show(ui, |p| {
                    p.hline(HLine::new(75.0).color(BAD.gamma_multiply(0.8)).name("75 kHz"));
                    p.line(Line::new(PlotPoints::new(pts)).color(TRACE_DEV).width(1.2).fill(0.0));
                });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if !self.error.is_empty() {
                ui.label(RichText::new(&self.error).color(BAD));
            } else {
                let st = self.shared.status.lock().unwrap().clone();
                let st = if st.is_empty() { "Not connected".to_string() } else { st };
                ui.label(RichText::new(st).size(11.5).color(TEXT_DIM));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match &self.sound {
                    Some(snd) => ui.label(
                        RichText::new(format!("Audio: {} · {} Hz", snd.device_name, snd.rate))
                            .size(11.5)
                            .color(TEXT_DIM),
                    ),
                    None => ui.label(RichText::new(&self.sound_error).size(11.5).color(WARN)),
                };
            });
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(s) = self.shared.snapshot.lock().unwrap().take() {
            self.peak_hold = self.peak_hold.max(s.peak_khz);
            self.snap = Some(s);
        }
        // Worker ended on its own (error / end of stream)?
        if self.worker.is_some() && !self.running() {
            let st = self.shared.status.lock().unwrap().clone();
            if st.contains("failed") || st.contains("error") || st.contains("ended") {
                self.stop();
                self.error = st;
            }
        }

        if !ctx.wants_keyboard_input() {
            let (l, r, u, d) = ctx.input(|i| {
                (
                    i.key_pressed(Key::ArrowLeft),
                    i.key_pressed(Key::ArrowRight),
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::ArrowDown),
                )
            });
            let step = if l || d { -0.1 } else if r || u { 0.1 } else { 0.0 };
            if step != 0.0 {
                self.step_freq(step);
            }
        }

        let bar = |fill| egui::Frame::none().fill(fill).inner_margin(Margin::symmetric(14.0, 8.0));

        egui::TopBottomPanel::top("header")
            .frame(bar(BG).stroke(Stroke::new(1.0, CARD_BORDER)))
            .show(ctx, |ui| self.header(ui));
        egui::TopBottomPanel::top("tuner")
            .frame(bar(PANEL).stroke(Stroke::new(1.0, CARD_BORDER)))
            .show(ctx, |ui| self.tuner_bar(ui));
        egui::TopBottomPanel::bottom("status")
            .frame(bar(BG).inner_margin(Margin::symmetric(14.0, 4.0)))
            .show(ctx, |ui| self.status_bar(ui));

        let side = egui::Frame::none().fill(BG).inner_margin(Margin::same(10.0));
        egui::SidePanel::left("meters")
            .frame(side)
            .resizable(false)
            .exact_width(340.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.meters(ui));
            });
        egui::SidePanel::right("rds")
            .frame(side)
            .resizable(false)
            .exact_width(320.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.rds_panel(ui));
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(Margin::symmetric(0.0, 10.0)))
            .show(ctx, |ui| self.plots(ui));

        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("{APP_NAME} v{}", env!("CARGO_PKG_VERSION")))
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1180.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
