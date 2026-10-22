//! FM broadcast analyzer: channel filter, FM discriminator, MPX measurements
//! (peak deviation, pilot, RDS injection, ITU-R BS.412 MPX power), spectra and RDS.

use crate::audio::StereoDecoder;
use crate::dsp::{lowpass_taps, DecimFirC, PilotPll, Spectrum, C32};
use crate::rds::{RdsDemod, RdsState};
use std::collections::VecDeque;
use std::f32::consts::PI;

/// RTL-SDR sample rate used by the analyzer.
pub const IQ_RATE: u32 = 2_280_000;
/// Decimation from IQ to MPX rate.
pub const DECIM: usize = 10;
/// MPX sample rate (4 x 57 kHz).
pub const MPX_RATE: f32 = (IQ_RATE as usize / DECIM) as f32;
/// MPX samples per measurement window (100 ms).
const WINDOW: usize = (MPX_RATE as usize) / 10;
const RF_FFT: usize = 2048;
const MPX_FFT: usize = 4096;
/// History length of the deviation trace (60 s of 100 ms windows).
pub const HISTORY: usize = 600;
/// Reference power for BS.412: a sine with +-19 kHz deviation.
const MPX_REF_POWER: f64 = 19_000.0 * 19_000.0 / 2.0;

/// One measurement update, produced every 100 ms.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Power of the filtered FM channel, dBFS.
    pub channel_dbfs: f32,
    /// Power of the whole 2.28 MHz band, dBFS.
    pub band_dbfs: f32,
    /// Peak deviation in this window, kHz.
    pub peak_khz: f32,
    /// Percentage of samples above 75 kHz deviation.
    pub over75_pct: f32,
    /// Mean carrier offset (DC of the discriminator), kHz.
    pub offset_khz: f32,
    pub pilot_khz: f32,
    pub pilot_locked: bool,
    pub pilot_freq_hz: f32,
    pub rds_khz: f32,
    /// MPX power over the last 100 ms window, dBr (BS.412).
    pub mpx_power_inst: f32,
    /// MPX power integrated over the last 60 s, dBr (BS.412).
    pub mpx_power_60s: f32,
    /// Seconds integrated in `mpx_power_60s` (up to 60).
    pub mpx_seconds: usize,
    /// Stereo decoder active (pilot locked and not forced mono).
    pub stereo: bool,
    /// Audio peak levels over the window, dBFS (after de-emphasis).
    pub audio_l_db: f32,
    pub audio_r_db: f32,
    /// RF spectrum (dBFS), FFT-shifted, `RF_FFT` bins across `IQ_RATE`.
    pub rf_db: Vec<f32>,
    /// MPX spectrum, dB relative to 1 kHz deviation, bins 0..MPX_RATE/2.
    pub mpx_db: Vec<f32>,
    /// Peak deviation history, kHz, oldest first.
    pub dev_history: Vec<f32>,
    pub rds: RdsState,
}

impl Snapshot {
    pub fn rf_bin_hz(&self) -> f32 {
        IQ_RATE as f32 / self.rf_db.len().max(1) as f32
    }
    pub fn mpx_bin_hz(&self) -> f32 {
        MPX_RATE / (2 * self.mpx_db.len()).max(1) as f32
    }
}

pub struct Analyzer {
    chan: DecimFirC,
    prev: C32,
    pll: PilotPll,
    rds_fir: DecimFirC,
    rds: RdsDemod,
    rf_spec: Spectrum,
    mpx_spec: Spectrum,
    raw_ring: Vec<C32>,
    raw_pos: usize,
    mpx_ring: Vec<f32>,
    mpx_pos: usize,
    carry: Option<u8>,
    // window accumulators
    win_n: usize,
    win_peak: f32,
    win_sq: f64,
    win_sum: f64,
    win_over: usize,
    win_chan_pow: f64,
    raw_pow: f64,
    raw_n: usize,
    // BS.412
    sec_sq: f64,
    sec_n: usize,
    seconds: VecDeque<f64>,
    history: VecDeque<f32>,
    // audio
    stereo: StereoDecoder,
    audio: Vec<[f32; 2]>,
    audio_enabled: bool,
    peak_l: f32,
    peak_r: f32,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        let fs = IQ_RATE as f32;
        Self {
            // +-110 kHz channel filter, then decimate to 228 kS/s
            chan: DecimFirC::new(lowpass_taps(255, 110_000.0 / fs), DECIM),
            prev: C32::default(),
            pll: PilotPll::new(MPX_RATE, 19_000.0),
            // RDS baseband: low-pass ~2.6 kHz, decimate by 12 to 19 kS/s
            rds_fir: DecimFirC::new(lowpass_taps(241, 2_600.0 / MPX_RATE), 12),
            rds: RdsDemod::new(),
            rf_spec: Spectrum::new(RF_FFT, 0.3),
            mpx_spec: Spectrum::new(MPX_FFT, 0.3),
            raw_ring: vec![C32::default(); RF_FFT],
            raw_pos: 0,
            mpx_ring: vec![0.0; MPX_FFT],
            mpx_pos: 0,
            carry: None,
            win_n: 0,
            win_peak: 0.0,
            win_sq: 0.0,
            win_sum: 0.0,
            win_over: 0,
            win_chan_pow: 0.0,
            raw_pow: 0.0,
            raw_n: 0,
            sec_sq: 0.0,
            sec_n: 0,
            seconds: VecDeque::with_capacity(61),
            history: VecDeque::with_capacity(HISTORY + 1),
            stereo: StereoDecoder::new(MPX_RATE, 50.0),
            audio: Vec::with_capacity(8192),
            audio_enabled: true,
            peak_l: 0.0,
            peak_r: 0.0,
        }
    }

    /// Call after retuning: clears station-dependent state (RDS, MPX power, history).
    pub fn reset_station(&mut self) {
        self.rds.reset();
        self.seconds.clear();
        self.sec_sq = 0.0;
        self.sec_n = 0;
        self.history.clear();
        self.mpx_spec.reset();
        self.pll = PilotPll::new(MPX_RATE, 19_000.0);
    }

    /// De-emphasis time constant in microseconds (50 Europe, 75 Americas, 0 = off).
    pub fn set_deemphasis(&mut self, tau_us: f32) {
        self.stereo.set_deemphasis(tau_us);
    }

    pub fn set_force_mono(&mut self, mono: bool) {
        self.stereo.force_mono = mono;
    }

    /// When disabled, decoded audio is not collected (meters still work).
    pub fn set_audio_enabled(&mut self, on: bool) {
        self.audio_enabled = on;
        if !on {
            self.audio.clear();
        }
    }

    /// Takes the L/R frames (at [`crate::audio::AUDIO_RATE`]) decoded so far.
    pub fn drain_audio(&mut self, out: &mut Vec<[f32; 2]>) {
        out.append(&mut self.audio);
    }

    pub fn rds_state(&self) -> &RdsState {
        &self.rds.decoder.state
    }

    /// Feed interleaved unsigned 8-bit I/Q (RTL-SDR format).
    /// Returns the snapshots completed while processing this buffer (usually 0 or 1).
    pub fn process(&mut self, data: &[u8]) -> Vec<Snapshot> {
        let mut out = Vec::new();
        let mut rest = data;
        if let Some(c) = self.carry.take() {
            if let Some((&q, tail)) = rest.split_first() {
                if let Some(s) = self.process_sample(c, q) {
                    out.push(s);
                }
                rest = tail;
            } else {
                self.carry = Some(c);
                return out;
            }
        }
        let mut chunks = rest.chunks_exact(2);
        for p in &mut chunks {
            if let Some(s) = self.process_sample(p[0], p[1]) {
                out.push(s);
            }
        }
        if let [last] = chunks.remainder() {
            self.carry = Some(*last);
        }
        out
    }

    #[inline]
    fn process_sample(&mut self, bi: u8, bq: u8) -> Option<Snapshot> {
        let i = (bi as f32 - 127.4) * (1.0 / 128.0);
        let q = (bq as f32 - 127.4) * (1.0 / 128.0);
        let raw = C32::new(i, q);
        self.raw_ring[self.raw_pos] = raw;
        self.raw_pos = (self.raw_pos + 1) % RF_FFT;
        self.raw_pow += raw.norm_sqr() as f64;
        self.raw_n += 1;

        let z = self.chan.push(i, q)?;
        self.win_chan_pow += z.norm_sqr() as f64;
        let d = z * self.prev.conj();
        self.prev = z;
        // instantaneous frequency in Hz == MPX in Hz of deviation
        let f = d.arg() * (MPX_RATE / (2.0 * PI));
        self.process_mpx(f)
    }

    #[inline]
    fn process_mpx(&mut self, f: f32) -> Option<Snapshot> {
        self.mpx_ring[self.mpx_pos] = f;
        self.mpx_pos = (self.mpx_pos + 1) % MPX_FFT;

        let a = f.abs();
        if a > self.win_peak {
            self.win_peak = a;
        }
        if a > 75_000.0 {
            self.win_over += 1;
        }
        let f64v = f as f64;
        self.win_sum += f64v;
        self.win_sq += f64v * f64v;
        self.sec_sq += f64v * f64v;
        self.sec_n += 1;
        if self.sec_n >= MPX_RATE as usize {
            self.seconds.push_back(self.sec_sq / self.sec_n as f64);
            if self.seconds.len() > 60 {
                self.seconds.pop_front();
            }
            self.sec_sq = 0.0;
            self.sec_n = 0;
        }

        // phase of the pilot at *this* sample (step() advances it to the next one)
        let ph = self.pll.phase;
        self.pll.step(f);
        let locked = self.pll.locked();
        if let Some(lr) = self.stereo.push(f, ph, locked) {
            self.peak_l = self.peak_l.max(lr[0].abs());
            self.peak_r = self.peak_r.max(lr[1].abs());
            if self.audio_enabled {
                self.audio.push(lr);
            }
        }
        let (s3, c3) = (3.0 * ph).sin_cos();
        if let Some(zr) = self.rds_fir.push(f * c3, -f * s3) {
            self.rds.push(zr);
        }

        self.win_n += 1;
        if self.win_n >= WINDOW {
            Some(self.finish_window())
        } else {
            None
        }
    }

    fn finish_window(&mut self) -> Snapshot {
        let n = self.win_n as f64;
        let mean = self.win_sum / n;
        // MPX power excludes the DC term (carrier offset)
        let var = (self.win_sq / n - mean * mean).max(1e-9);

        let raw = &self.raw_ring;
        let rp = self.raw_pos;
        self.rf_spec
            .update((0..RF_FFT).map(|k| raw[(rp + k) % RF_FFT]));
        let mr = &self.mpx_ring;
        let mp = self.mpx_pos;
        self.mpx_spec
            .update((0..MPX_FFT).map(|k| C32::new(mr[(mp + k) % MPX_FFT] - mean as f32, 0.0)));

        let p = self.rf_spec.power();
        let half = RF_FFT / 2;
        let rf_db: Vec<f32> = (0..RF_FFT)
            .map(|k| 10.0 * (p[(k + half) % RF_FFT] + 1e-20).log10())
            .collect();
        // real tone amplitude A -> |X|/sum(w) = A/2 ; express relative to 1 kHz
        let mpx_db: Vec<f32> = self.mpx_spec.power()[..MPX_FFT / 2]
            .iter()
            .map(|&pw| 10.0 * (4.0 * pw / 1.0e6 + 1e-20).log10())
            .collect();

        let peak_khz = self.win_peak / 1000.0;
        self.history.push_back(peak_khz);
        if self.history.len() > HISTORY {
            self.history.pop_front();
        }

        let mpx60 = if self.seconds.is_empty() {
            var
        } else {
            self.seconds.iter().sum::<f64>() / self.seconds.len() as f64
        };

        let snap = Snapshot {
            channel_dbfs: 10.0 * ((self.win_chan_pow / n) as f32 + 1e-20).log10(),
            band_dbfs: 10.0 * ((self.raw_pow / self.raw_n.max(1) as f64) as f32 + 1e-20).log10(),
            peak_khz,
            over75_pct: 100.0 * self.win_over as f32 / self.win_n as f32,
            offset_khz: (mean / 1000.0) as f32,
            pilot_khz: self.pll.amp / 1000.0,
            pilot_locked: self.pll.locked(),
            pilot_freq_hz: self.pll.freq_hz(MPX_RATE),
            // shaped biphase: RMS ~ peak/sqrt(2); 57k mixing halves the amplitude
            rds_khz: 2.0 * std::f32::consts::SQRT_2 * self.rds.power.sqrt() / 1000.0,
            mpx_power_inst: (10.0 * (var / MPX_REF_POWER).log10()) as f32,
            mpx_power_60s: (10.0 * (mpx60 / MPX_REF_POWER).log10()) as f32,
            mpx_seconds: self.seconds.len(),
            stereo: self.pll.locked() && !self.stereo.force_mono,
            audio_l_db: 20.0 * (self.peak_l + 1e-6).log10(),
            audio_r_db: 20.0 * (self.peak_r + 1e-6).log10(),
            rf_db,
            mpx_db,
            dev_history: self.history.iter().copied().collect(),
            rds: self.rds.decoder.state.clone(),
        };

        self.win_n = 0;
        self.win_peak = 0.0;
        self.win_sq = 0.0;
        self.win_sum = 0.0;
        self.win_over = 0;
        self.win_chan_pow = 0.0;
        self.raw_pow = 0.0;
        self.raw_n = 0;
        self.peak_l = 0.0;
        self.peak_r = 0.0;
        snap
    }
}
