#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, RichText};
use egui_plot::{Line, Plot, PlotPoints, VLine};
use fm_analyzer::fm::{Snapshot, HISTORY};
use fm_analyzer::source::demo::{DemoSource, DEMO_FREQ_HZ};
use fm_analyzer::source::tcp::RtlTcpSource;
use fm_analyzer::source::{IqSource, R820T_GAINS};
use fm_analyzer::worker::{Cmd, Shared, Tuning, Worker};
use std::sync::Arc;
use std::time::Duration;

const APP_NAME: &str = "FM Analyzer";

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Usb,
    Tcp,
    Demo,
}

struct App {
    kind: SourceKind,
    host: String,
    port: u16,
    usb_index: u32,
    usb_devices: Vec<String>,
    freq_mhz: f64,
    gain_auto: bool,
    gain_idx: usize,
    ppm: i32,
    shared: Arc<Shared>,
    worker: Option<Worker>,
    snap: Option<Snapshot>,
    peak_hold: f32,
    error: String,
}

impl App {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            kind: if cfg!(feature = "usb") { SourceKind::Usb } else { SourceKind::Tcp },
            host: "127.0.0.1".into(),
            port: 1234,
            usb_index: 0,
            usb_devices: Vec::new(),
            freq_mhz: DEMO_FREQ_HZ as f64 / 1e6,
            gain_auto: true,
            gain_idx: 16,
            ppm: 0,
            shared: Arc::new(Shared::default()),
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
            SourceKind::Demo => Ok(Box::new(DemoSource::new(true))),
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
        match self.open_source() {
            Ok(src) => {
                let ctx = ctx.clone();
                let tuning = Tuning { freq_hz: self.freq_hz(), gain: self.gain(), ppm: self.ppm };
                self.worker = Some(Worker::spawn(
                    src,
                    tuning,
                    self.shared.clone(),
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
    }

    fn send(&self, cmd: Cmd) {
        if let Some(w) = &self.worker {
            w.send(cmd);
        }
    }

    fn retune(&mut self) {
        self.freq_mhz = self.freq_mhz.clamp(64.0, 108.0);
        self.peak_hold = 0.0;
        self.send(Cmd::SetFreq(self.freq_hz()));
    }

    fn running(&self) -> bool {
        *self.shared.running.lock().unwrap()
    }

    // ---------------------------------------------------------------- UI parts

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Source:");
            #[cfg(feature = "usb")]
            ui.selectable_value(&mut self.kind, SourceKind::Usb, "RTL-SDR USB");
            ui.selectable_value(&mut self.kind, SourceKind::Tcp, "rtl_tcp");
            ui.selectable_value(&mut self.kind, SourceKind::Demo, "Demo");
            ui.separator();

            match self.kind {
                SourceKind::Tcp => {
                    ui.label("Host");
                    ui.add(egui::TextEdit::singleline(&mut self.host).desired_width(120.0));
                    ui.label("Port");
                    ui.add(egui::DragValue::new(&mut self.port));
                }
                SourceKind::Usb => {
                    ui.label("Device");
                    ui.add(egui::DragValue::new(&mut self.usb_index));
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
                    if !self.usb_devices.is_empty() {
                        ui.label(self.usb_devices.join(", "));
                    }
                }
                SourceKind::Demo => {
                    ui.label("Built-in test station on 98.00 MHz");
                }
            }
            ui.separator();
            let ctx = ui.ctx().clone();
            if self.worker.is_some() {
                if ui.button("⏹ Stop").clicked() {
                    self.stop();
                }
            } else if ui.button("▶ Start").clicked() {
                self.start(&ctx);
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Frequency");
            if ui.button("◀◀").on_hover_text("-1 MHz").clicked() {
                self.freq_mhz -= 1.0;
                self.retune();
            }
            if ui.button("◀").on_hover_text("-100 kHz (Left arrow)").clicked() {
                self.freq_mhz -= 0.1;
                self.retune();
            }
            let r = ui.add(
                egui::DragValue::new(&mut self.freq_mhz)
                    .speed(0.05)
                    .fixed_decimals(2)
                    .suffix(" MHz"),
            );
            if r.drag_stopped() || r.lost_focus() {
                self.freq_mhz = (self.freq_mhz * 20.0).round() / 20.0; // 50 kHz raster
                self.retune();
            }
            if ui.button("▶").on_hover_text("+100 kHz (Right arrow)").clicked() {
                self.freq_mhz += 0.1;
                self.retune();
            }
            if ui.button("▶▶").on_hover_text("+1 MHz").clicked() {
                self.freq_mhz += 1.0;
                self.retune();
            }
            ui.separator();
            if ui.checkbox(&mut self.gain_auto, "Auto gain").changed() {
                self.send(Cmd::SetGain(self.gain()));
            }
            ui.add_enabled_ui(!self.gain_auto, |ui| {
                let max = R820T_GAINS.len() - 1;
                let label = format!("{:.1} dB", R820T_GAINS[self.gain_idx]);
                let r = ui.add(
                    egui::Slider::new(&mut self.gain_idx, 0..=max)
                        .show_value(false)
                        .text(label),
                );
                if r.changed() {
                    self.send(Cmd::SetGain(self.gain()));
                }
            });
            ui.separator();
            ui.label("PPM");
            if ui.add(egui::DragValue::new(&mut self.ppm)).changed() {
                self.ppm = self.ppm.clamp(-200, 200);
                self.send(Cmd::SetPpm(self.ppm));
            }
            ui.separator();
            if ui.button("Reset RDS").clicked() {
                self.peak_hold = 0.0;
                self.send(Cmd::ResetRds);
            }
        });
    }

    fn meters(&mut self, ui: &mut egui::Ui) {
        let Some(s) = &self.snap else {
            ui.label("No data. Choose a source and press Start.");
            return;
        };
        let big = |t: String, c: Color32| RichText::new(t).size(22.0).monospace().color(c);
        let ok = Color32::from_rgb(80, 200, 120);
        let warn = Color32::from_rgb(240, 180, 60);
        let bad = Color32::from_rgb(235, 80, 70);
        let dim = ui.visuals().weak_text_color();

        ui.heading(format!("{:.2} MHz", self.freq_mhz));
        egui::Grid::new("meters").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Channel level");
            ui.label(big(format!("{:6.1} dBFS", s.channel_dbfs), if s.channel_dbfs > -3.0 { bad } else { ok }));
            ui.end_row();

            ui.label("Peak deviation");
            let c = if s.peak_khz > 75.0 { bad } else if s.peak_khz > 70.0 { warn } else { ok };
            ui.label(big(format!("{:6.1} kHz", s.peak_khz), c));
            ui.end_row();

            ui.label("Peak hold");
            ui.label(big(format!("{:6.1} kHz", self.peak_hold), if self.peak_hold > 75.0 { bad } else { ok }));
            ui.end_row();

            ui.label("> 75 kHz");
            ui.label(big(format!("{:6.2} %", s.over75_pct), if s.over75_pct > 0.0 { warn } else { ok }));
            ui.end_row();

            ui.label("Pilot 19 kHz");
            let (txt, c) = if s.pilot_locked {
                (format!("{:6.2} kHz", s.pilot_khz), if (6.0..=7.5).contains(&s.pilot_khz) { ok } else { warn })
            } else {
                ("  ----- mono".to_string(), dim)
            };
            ui.label(big(txt, c));
            ui.end_row();

            if s.pilot_locked {
                ui.label("Pilot freq");
                ui.label(RichText::new(format!("{:.2} Hz", s.pilot_freq_hz)).monospace());
                ui.end_row();
            }

            ui.label("RDS (approx.)");
            let rds_ok = s.rds.synced;
            ui.label(big(format!("{:6.2} kHz", s.rds_khz), if rds_ok { ok } else { dim }));
            ui.end_row();

            ui.label("MPX power 60 s");
            let c = if s.mpx_power_60s > 0.0 { bad } else { ok };
            ui.label(big(format!("{:+6.2} dBr", s.mpx_power_60s), c))
                .on_hover_text(format!("ITU-R BS.412, integrated over {} s", s.mpx_seconds));
            ui.end_row();

            ui.label("MPX power 100 ms");
            ui.label(RichText::new(format!("{:+.2} dBr", s.mpx_power_inst)).monospace());
            ui.end_row();

            ui.label("Carrier offset");
            ui.label(RichText::new(format!("{:+.2} kHz", s.offset_khz)).monospace());
            ui.end_row();
        });

        ui.add_space(8.0);
        let fill = (s.peak_khz / 100.0).clamp(0.0, 1.0);
        ui.add(
            egui::ProgressBar::new(fill)
                .text(format!("Deviation {:.1} / 75 kHz", s.peak_khz))
                .fill(if s.peak_khz > 75.0 { bad } else { ok }),
        );

        ui.separator();
        ui.heading("RDS");
        let r = &s.rds;
        ui.label(
            RichText::new(r.ps_string())
                .size(34.0)
                .monospace()
                .strong()
                .color(if r.synced { ok } else { dim }),
        );
        egui::Grid::new("rds").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label("PI");
            ui.label(RichText::new(r.pi.map(|p| format!("{p:04X}")).unwrap_or("----".into())).monospace().strong());
            ui.end_row();
            ui.label("PTY");
            ui.label(format!("{} ({})", r.pty_name(), r.pty.map(|p| p.to_string()).unwrap_or("-".into())));
            ui.end_row();
            ui.label("Flags");
            let flag = |on: bool, n: &str| if on { n.to_string() } else { n.to_lowercase() };
            ui.label(RichText::new(format!(
                "{}  {}  {}",
                flag(r.tp, "TP"),
                flag(r.ta, "TA"),
                if r.music { "Music" } else { "Speech" }
            )).monospace());
            ui.end_row();
            ui.label("CT");
            ui.label(r.clock.clone().unwrap_or("-".into()));
            ui.end_row();
            ui.label("AF");
            let af: Vec<String> = r.af.iter().map(|f| format!("{:.1}", *f as f32 / 10.0)).collect();
            ui.label(if af.is_empty() { "-".to_string() } else { af.join(" ") });
            ui.end_row();
            ui.label("Sync / BLER");
            ui.label(format!(
                "{}  {:.1} %  ({} groups)",
                if r.synced { "synced" } else { "searching" },
                r.bler(),
                r.total_groups()
            ));
            ui.end_row();
        });
        ui.label("Radiotext:");
        ui.label(RichText::new(r.rt_string()).monospace());

        ui.collapsing("Group statistics", |ui| {
            egui::Grid::new("groups").striped(true).show(ui, |ui| {
                for (t, g) in r.groups.iter().enumerate() {
                    for (v, n) in g.iter().enumerate() {
                        if *n > 0 {
                            ui.label(format!("{t}{}", if v == 0 { 'A' } else { 'B' }));
                            ui.label(n.to_string());
                            ui.end_row();
                        }
                    }
                }
            });
        });
    }

    fn plots(&self, ui: &mut egui::Ui) {
        let Some(s) = &self.snap else { return };
        let h = ((ui.available_height() - 60.0) / 3.0).max(120.0);
        let center = self.freq_mhz;

        ui.label(RichText::new("RF spectrum (dBFS)").strong());
        let n = s.rf_db.len();
        let bin = s.rf_bin_hz() as f64;
        let rf: Vec<[f64; 2]> = s
            .rf_db
            .iter()
            .enumerate()
            .map(|(k, &db)| {
                [center + (k as f64 - n as f64 / 2.0) * bin / 1e6, db.clamp(-110.0, 0.0) as f64]
            })
            .collect();
        Plot::new("rf")
            .height(h)
            .include_y(-110.0)
            .include_y(0.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .x_axis_label("MHz")
            .show(ui, |p| {
                p.line(Line::new(PlotPoints::new(rf)).color(Color32::from_rgb(90, 170, 255)));
                p.vline(VLine::new(center).color(Color32::from_rgba_unmultiplied(255, 255, 255, 60)));
                for off in [-0.1, 0.1] {
                    p.vline(VLine::new(center + off).color(Color32::from_rgba_unmultiplied(255, 200, 0, 50)));
                }
            });

        ui.label(RichText::new("MPX spectrum (dB re 1 kHz deviation)").strong());
        let bin = s.mpx_bin_hz() as f64;
        let mpx: Vec<[f64; 2]> = s
            .mpx_db
            .iter()
            .enumerate()
            .map(|(k, &db)| [k as f64 * bin / 1000.0, db.clamp(-60.0, 40.0) as f64])
            .collect();
        Plot::new("mpx")
            .height(h)
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
                p.line(Line::new(PlotPoints::new(mpx)).color(Color32::from_rgb(120, 220, 140)));
                for (f, name) in [(15.0, "15k"), (19.0, "Pilot"), (23.0, "23k"), (38.0, "38k"), (53.0, "53k"), (57.0, "RDS")] {
                    p.vline(
                        VLine::new(f)
                            .name(name)
                            .color(Color32::from_rgba_unmultiplied(255, 200, 0, 60)),
                    );
                }
            });

        ui.label(RichText::new("Peak deviation, last 60 s (kHz)").strong());
        let len = s.dev_history.len();
        let dev: Vec<[f64; 2]> = s
            .dev_history
            .iter()
            .enumerate()
            .map(|(k, &v)| [(k as f64 - len as f64) / 10.0, v.clamp(0.0, 150.0) as f64])
            .collect();
        Plot::new("dev")
            .height(h)
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
                p.line(Line::new(PlotPoints::new(dev)).color(Color32::from_rgb(255, 150, 90)));
                p.hline(egui_plot::HLine::new(75.0).color(Color32::from_rgb(235, 80, 70)).name("75 kHz"));
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
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::ArrowDown),
                )
            });
            let step = if l || d { -0.1 } else if r || u { 0.1 } else { 0.0 };
            if step != 0.0 {
                self.freq_mhz = ((self.freq_mhz + step) * 10.0).round() / 10.0;
                self.retune();
            }
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if !self.error.is_empty() {
                    ui.colored_label(Color32::from_rgb(235, 80, 70), &self.error);
                } else {
                    ui.label(self.shared.status.lock().unwrap().as_str());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{APP_NAME} v{}", env!("CARGO_PKG_VERSION")));
                });
            });
        });
        egui::SidePanel::left("meters")
            .resizable(true)
            .default_width(360.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.meters(ui));
            });
        egui::CentralPanel::default().show(ctx, |ui| self.plots(ui));

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
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
