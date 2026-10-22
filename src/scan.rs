//! FM band scanner.
//!
//! Phase 1 (levels): the band is covered in hops of 1.6 MHz. At each hop the tuner settles,
//! then a Welch-averaged FFT gives the power of every 100 kHz channel within +-750 kHz of
//! the centre (the centre sits 50 kHz off the channel raster, so no channel lands on the
//! DC spike). Stations are local maxima that rise `threshold_db` above the noise floor.
//!
//! Phase 2 (optional, RDS): the scanner tunes to each station for a short dwell and runs
//! the full analyzer to read PI / PS / PTY, stereo pilot and peak deviation.
//!
//! [`ScanJob`] is a pure state machine: feed it I/Q bytes, and retune when it asks.

use crate::dsp::{Fft, C32};
use crate::fm::{Analyzer, IQ_RATE};
use std::f32::consts::PI;

pub const RASTER_KHZ: u32 = 100;
const HOP_HALF_KHZ: u32 = 750;
const HOP_STEP_KHZ: u32 = 2 * HOP_HALF_KHZ + RASTER_KHZ; // 1.6 MHz
const FFT_N: usize = 4096;
const CAPTURE_FFTS: usize = 48;
/// Integration bandwidth per channel, +-Hz.
const CH_HALF_BW: f32 = 80_000.0;
const BYTES_PER_SEC: f32 = IQ_RATE as f32 * 2.0;

#[derive(Clone, Copy, Debug)]
pub struct ScanConfig {
    pub start_khz: u32,
    pub stop_khz: u32,
    /// Minimum height above the noise floor for a station, dB.
    pub threshold_db: f32,
    /// Visit each station to read RDS, pilot and deviation.
    pub rds: bool,
    /// Maximum time per station in the RDS phase, seconds.
    pub dwell_s: f32,
    /// Time to discard after each retune (tuner PLL + rtl_tcp buffers), seconds.
    pub settle_s: f32,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            start_khz: 87_500,
            stop_khz: 108_000,
            threshold_db: 10.0,
            rds: true,
            dwell_s: 4.0,
            settle_s: 0.3,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StationInfo {
    pub freq_khz: u32,
    /// Channel power, dBFS (+-80 kHz).
    pub level_dbfs: f32,
    /// Height above the band noise floor, dB.
    pub cn_db: f32,
    pub rds_checked: bool,
    pub pi: Option<u16>,
    pub ps: Option<String>,
    pub pty: Option<u8>,
    pub stereo: Option<bool>,
    pub peak_dev_khz: Option<f32>,
}

#[derive(Clone, Debug, Default)]
pub struct ScanState {
    pub running: bool,
    /// 0..1
    pub progress: f32,
    pub status: String,
    /// Every channel of the band: (kHz, dBFS).
    pub band: Vec<(u32, f32)>,
    pub noise_dbfs: f32,
    pub stations: Vec<StationInfo>,
    /// Tuner gain used for the scan, dB.
    pub gain_db: f32,
    /// Index of the station being probed in the RDS phase.
    pub probing: Option<usize>,
}

enum Phase {
    Settle { left: usize, then_rds: Option<usize> },
    Capture,
    Rds { idx: usize, fed: usize },
    Done,
}

pub struct ScanJob {
    cfg: ScanConfig,
    hops: Vec<u32>,
    hop: usize,
    phase: Phase,
    fft: Fft,
    window: Vec<f32>,
    win_pow_sum: f32,
    acc: Vec<f64>,
    nacc: usize,
    block: Vec<u8>,
    work: Vec<C32>,
    analyzer: Option<Analyzer>,
    best_dev: f32,
    pub state: ScanState,
}

/// Hop centre frequencies (kHz) covering `start..=stop` on the 100 kHz raster.
pub fn hop_centers(start_khz: u32, stop_khz: u32) -> Vec<u32> {
    let mut v = Vec::new();
    let mut c = start_khz + HOP_HALF_KHZ;
    loop {
        v.push(c);
        if c + HOP_HALF_KHZ >= stop_khz {
            break;
        }
        c += HOP_STEP_KHZ;
    }
    v
}

/// Indices of `band` entries that are stations: local maxima at least `threshold_db`
/// above `noise_dbfs`.
pub fn find_stations(band: &[(u32, f32)], noise_dbfs: f32, threshold_db: f32) -> Vec<usize> {
    let mut out = Vec::new();
    for i in 0..band.len() {
        let l = band[i].1;
        if l < noise_dbfs + threshold_db {
            continue;
        }
        let left = if i > 0 { band[i - 1].1 } else { f32::MIN };
        let right = if i + 1 < band.len() { band[i + 1].1 } else { f32::MIN };
        if l > left && l >= right {
            out.push(i);
        }
    }
    out
}

/// Noise floor estimate: 20th percentile of all channel levels.
pub fn noise_floor(band: &[(u32, f32)]) -> f32 {
    if band.is_empty() {
        return -100.0;
    }
    let mut v: Vec<f32> = band.iter().map(|b| b.1).collect();
    let idx = v.len() / 5;
    let (_, nth, _) = v.select_nth_unstable_by(idx, |a, b| a.total_cmp(b));
    *nth
}

impl ScanJob {
    pub fn new(cfg: ScanConfig, gain_db: f32) -> Self {
        let window: Vec<f32> = (0..FFT_N)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FFT_N as f32).cos())
            .collect();
        let win_pow_sum = window.iter().map(|w| w * w).sum();
        let cfg = ScanConfig {
            stop_khz: cfg.stop_khz.max(cfg.start_khz),
            ..cfg
        };
        let hops = hop_centers(cfg.start_khz, cfg.stop_khz);
        let settle = (cfg.settle_s * BYTES_PER_SEC) as usize & !1;
        Self {
            cfg,
            hops,
            hop: 0,
            phase: Phase::Settle { left: settle, then_rds: None },
            fft: Fft::new(FFT_N),
            window,
            win_pow_sum,
            acc: vec![0.0; FFT_N],
            nacc: 0,
            block: Vec::with_capacity(FFT_N * 2),
            work: vec![C32::default(); FFT_N],
            analyzer: None,
            best_dev: 0.0,
            state: ScanState {
                running: true,
                status: "Measuring band levels".into(),
                gain_db,
                ..Default::default()
            },
        }
    }

    /// Frequency (Hz) the caller must tune to before feeding data.
    pub fn first_freq_hz(&self) -> u32 {
        self.hops[0] * 1000
    }

    pub fn finished(&self) -> bool {
        matches!(self.phase, Phase::Done)
    }

    fn settle_bytes(&self) -> usize {
        (self.cfg.settle_s * BYTES_PER_SEC) as usize & !1
    }

    fn levels_weight(&self) -> f32 {
        if self.cfg.rds {
            0.35
        } else {
            1.0
        }
    }

    /// Stops the scan, keeping partial results.
    pub fn stop(&mut self) {
        if !self.finished() {
            self.phase = Phase::Done;
            self.state.running = false;
            self.state.probing = None;
            self.state.status = format!("Stopped · {} stations", self.state.stations.len());
        }
    }

    /// Feeds raw u8 I/Q. Returns `Some(hz)` when the caller must retune; the rest of
    /// `data` is then ignored (it belongs to the old frequency).
    pub fn feed(&mut self, data: &[u8]) -> Option<u32> {
        let mut data = data;
        loop {
            if data.is_empty() {
                return None;
            }
            match &mut self.phase {
                Phase::Done => return None,
                Phase::Settle { left, then_rds } => {
                    let take = (*left).min(data.len()) & !1;
                    let take = if take == 0 { data.len().min(*left) } else { take };
                    *left -= take;
                    data = &data[take..];
                    if *left == 0 {
                        self.phase = match then_rds.take() {
                            Some(idx) => {
                                let mut a = Analyzer::new();
                                a.set_audio_enabled(false);
                                self.analyzer = Some(a);
                                self.best_dev = 0.0;
                                Phase::Rds { idx, fed: 0 }
                            }
                            None => {
                                self.block.clear();
                                self.acc.iter_mut().for_each(|a| *a = 0.0);
                                self.nacc = 0;
                                Phase::Capture
                            }
                        };
                    }
                }
                Phase::Capture => {
                    let need = FFT_N * 2 - self.block.len();
                    let take = need.min(data.len());
                    self.block.extend_from_slice(&data[..take]);
                    data = &data[take..];
                    if self.block.len() == FFT_N * 2 {
                        self.accumulate_block();
                        self.block.clear();
                        if self.nacc >= CAPTURE_FFTS {
                            if let Some(f) = self.finish_hop() {
                                return Some(f);
                            }
                        }
                    }
                }
                Phase::Rds { idx, fed } => {
                    let idx = *idx;
                    *fed += data.len();
                    let fed_now = *fed;
                    let a = self.analyzer.as_mut().expect("analyzer");
                    let warm = fed_now as f32 / BYTES_PER_SEC > 0.5; // skip filter start-up transient
                    for snap in a.process(data) {
                        if warm {
                            self.best_dev = self.best_dev.max(snap.peak_khz);
                        }
                        let st = &mut self.state.stations[idx];
                        st.stereo = Some(snap.pilot_locked);
                        if warm {
                            st.peak_dev_khz = Some(self.best_dev);
                        }
                    }
                    data = &[];
                    let secs = fed_now as f32 / BYTES_PER_SEC;
                    let r = a.rds_state();
                    let ps_done = r.pi.is_some() && r.ps_seen == 0x0F;
                    let hopeless = secs > 1.5 && !r.synced && r.pi.is_none();
                    let st = &mut self.state.stations[idx];
                    st.rds_checked = true;
                    st.pi = r.pi;
                    st.pty = r.pty;
                    if r.ps_seen == 0x0F {
                        st.ps = Some(r.ps_string());
                    }
                    let total = self.state.stations.len().max(1) as f32;
                    let w = self.levels_weight();
                    self.state.progress = w
                        + (1.0 - w) * ((idx as f32 + (secs / self.cfg.dwell_s).min(1.0)) / total);
                    if (ps_done && secs > 1.0) || hopeless || secs >= self.cfg.dwell_s {
                        if let Some(f) = self.next_station(idx + 1) {
                            return Some(f);
                        }
                    }
                }
            }
        }
    }

    fn accumulate_block(&mut self) {
        let mut mi = 0.0f32;
        let mut mq = 0.0f32;
        for k in 0..FFT_N {
            let i = (self.block[2 * k] as f32 - 127.4) / 128.0;
            let q = (self.block[2 * k + 1] as f32 - 127.4) / 128.0;
            self.work[k] = C32::new(i, q);
            mi += i;
            mq += q;
        }
        // remove the DC offset of the dongle
        let dc = C32::new(mi / FFT_N as f32, mq / FFT_N as f32);
        for k in 0..FFT_N {
            self.work[k] = (self.work[k] - dc).scale(self.window[k]);
        }
        self.fft.process(&mut self.work);
        for (a, x) in self.acc.iter_mut().zip(&self.work) {
            *a += x.norm_sqr() as f64;
        }
        self.nacc += 1;
    }

    fn finish_hop(&mut self) -> Option<u32> {
        let center = self.hops[self.hop];
        // power of the windowed signal per bin, normalised so that the sum over a band
        // equals the mean power of the signal in that band (Parseval)
        let norm = 1.0 / (self.nacc as f64 * FFT_N as f64 * self.win_pow_sum as f64);
        let bin_hz = IQ_RATE as f32 / FFT_N as f32;
        let half = (CH_HALF_BW / bin_hz).round() as i64;
        let lo = center.saturating_sub(HOP_HALF_KHZ).max(self.cfg.start_khz);
        let hi = (center + HOP_HALF_KHZ).min(self.cfg.stop_khz);
        let mut f = lo.div_ceil(RASTER_KHZ) * RASTER_KHZ;
        while f <= hi {
            let off_bins = ((f as f32 - center as f32) * 1000.0 / bin_hz).round() as i64;
            let mut p = 0.0f64;
            for b in off_bins - half..=off_bins + half {
                p += self.acc[b.rem_euclid(FFT_N as i64) as usize];
            }
            let db = 10.0 * (p * norm + 1e-20).log10() as f32;
            self.state.band.push((f, db));
            f += RASTER_KHZ;
        }
        self.hop += 1;
        let w = self.levels_weight();
        self.state.progress = w * self.hop as f32 / self.hops.len() as f32;

        if self.hop < self.hops.len() {
            self.phase = Phase::Settle { left: self.settle_bytes(), then_rds: None };
            return Some(self.hops[self.hop] * 1000);
        }

        // band complete: find stations
        self.state.band.sort_by_key(|b| b.0);
        self.state.band.dedup_by_key(|b| b.0);
        let noise = noise_floor(&self.state.band);
        self.state.noise_dbfs = noise;
        self.state.stations = find_stations(&self.state.band, noise, self.cfg.threshold_db)
            .into_iter()
            .map(|i| StationInfo {
                freq_khz: self.state.band[i].0,
                level_dbfs: self.state.band[i].1,
                cn_db: self.state.band[i].1 - noise,
                ..Default::default()
            })
            .collect();
        if self.cfg.rds && !self.state.stations.is_empty() {
            self.next_station(0)
        } else {
            self.complete();
            None
        }
    }

    fn next_station(&mut self, idx: usize) -> Option<u32> {
        self.analyzer = None;
        if idx >= self.state.stations.len() {
            self.complete();
            return None;
        }
        self.state.probing = Some(idx);
        self.state.status = format!(
            "Reading RDS {}/{} · {:.1} MHz",
            idx + 1,
            self.state.stations.len(),
            self.state.stations[idx].freq_khz as f32 / 1000.0
        );
        self.phase = Phase::Settle { left: self.settle_bytes(), then_rds: Some(idx) };
        Some(self.state.stations[idx].freq_khz * 1000)
    }

    fn complete(&mut self) {
        self.phase = Phase::Done;
        self.state.running = false;
        self.state.probing = None;
        self.state.progress = 1.0;
        self.state.status = format!("Done · {} stations", self.state.stations.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::IqSource;
    use crate::testsignal::*;

    #[test]
    fn hop_plan_covers_band() {
        let h = hop_centers(87_500, 108_000);
        assert_eq!(h[0], 88_250);
        assert!(h.len() == 13, "{h:?}");
        assert!(*h.last().unwrap() + HOP_HALF_KHZ >= 108_000);
        // every hop centre is 50 kHz off the 100 kHz raster
        assert!(h.iter().all(|c| c % 100 == 50));
    }

    #[test]
    fn finds_local_maxima() {
        let band = vec![(100, -80.0), (200, -60.0), (300, -50.0), (400, -62.0), (500, -80.0), (600, -79.0)];
        assert_eq!(find_stations(&band, -80.0, 10.0), vec![2]);
    }

    #[test]
    fn scans_test_station_with_rds() {
        let mut src = TestSignal::new(false);
        let cfg = ScanConfig { start_khz: 96_000, stop_khz: 100_000, ..Default::default() };
        let mut job = ScanJob::new(cfg, 30.0);
        src.set_center_freq(job.first_freq_hz()).unwrap();
        let mut buf = vec![0u8; 65_536];
        let mut guard = 0;
        while !job.finished() {
            let n = src.read(&mut buf).unwrap();
            if let Some(f) = job.feed(&buf[..n]) {
                src.set_center_freq(f).unwrap();
            }
            guard += 1;
            assert!(guard < 2_000, "scan did not finish");
        }
        let s = &job.state;
        println!("noise {:.1} dBFS, stations {:?}", s.noise_dbfs, s.stations);
        assert_eq!(s.band.len(), 41);
        assert_eq!(s.stations.len(), 1, "{:?}", s.stations);
        let st = &s.stations[0];
        assert_eq!(st.freq_khz, TEST_FREQ_HZ / 1000);
        assert!(st.cn_db > 20.0);
        assert_eq!(st.pi, Some(TEST_PI));
        assert_eq!(st.ps.as_deref(), Some(std::str::from_utf8(TEST_PS).unwrap()));
        assert_eq!(st.stereo, Some(true));
        let dev = st.peak_dev_khz.unwrap();
        assert!(dev > 45.0 && dev < 60.0, "dev {dev}");
        assert!((s.progress - 1.0).abs() < 1e-6);
    }
}
