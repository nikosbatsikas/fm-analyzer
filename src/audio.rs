//! Stereo decoder (MPX -> L/R audio), resampler and the lock-protected audio
//! FIFO shared with the sound-card callback. No audio-API dependency here;
//! the actual output device lives in the application (cpal).

use crate::dsp::{lowpass_taps, DecimFirC};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

/// Decimation from the MPX rate (228 kS/s) to the decoder's audio rate.
pub const AUDIO_DECIM: usize = 4;
/// Audio rate produced by the stereo decoder.
pub const AUDIO_RATE: f32 = 57_000.0;
/// Deviation (Hz) that maps to 0 dBFS on one audio channel (90 % of 75 kHz).
pub const FULL_SCALE_HZ: f32 = 67_500.0;

/// Pilot-locked stereo decoder with de-emphasis.
pub struct StereoDecoder {
    fir: DecimFirC,
    deemph_l: f32,
    deemph_r: f32,
    deemph_a: f32,
    pub force_mono: bool,
}

impl StereoDecoder {
    /// `tau_us`: de-emphasis time constant, 50 (Europe) or 75 (Americas, Korea).
    pub fn new(mpx_rate: f32, tau_us: f32) -> Self {
        // L+R and L-R are filtered together as the real/imag parts of one complex FIR:
        // 15 kHz passband, ~40 dB+ at the 19 kHz pilot.
        let taps = lowpass_taps(401, 17_000.0 / mpx_rate);
        let mut s = Self {
            fir: DecimFirC::new(taps, AUDIO_DECIM),
            deemph_l: 0.0,
            deemph_r: 0.0,
            deemph_a: 1.0,
            force_mono: false,
        };
        s.set_deemphasis(tau_us);
        s
    }

    pub fn set_deemphasis(&mut self, tau_us: f32) {
        self.deemph_a = if tau_us <= 0.0 {
            1.0
        } else {
            1.0 - (-1.0 / (AUDIO_RATE * tau_us * 1e-6)).exp()
        };
    }

    /// One MPX sample (Hz of deviation) plus the pilot PLL phase.
    /// Returns an L/R frame (full scale = 1.0) every `AUDIO_DECIM` inputs.
    #[inline]
    pub fn push(&mut self, mpx: f32, pilot_phase: f32, stereo: bool) -> Option<[f32; 2]> {
        // Standard pilot sin(wt), subcarrier S*sin(2wt). The PLL tracks the pilot as cos(phase),
        // so sin(2wt) = -sin(2*phase).
        let diff = if stereo && !self.force_mono {
            -2.0 * mpx * (2.0 * pilot_phase).sin()
        } else {
            0.0
        };
        let z = self.fir.push(mpx, diff)?;
        let l = (z.re + z.im) * (1.0 / FULL_SCALE_HZ);
        let r = (z.re - z.im) * (1.0 / FULL_SCALE_HZ);
        self.deemph_l += self.deemph_a * (l - self.deemph_l);
        self.deemph_r += self.deemph_a * (r - self.deemph_r);
        Some([self.deemph_l, self.deemph_r])
    }
}

/// 4-point cubic Hermite resampler for interleaved stereo frames.
pub struct Resampler {
    hist: [[f32; 2]; 4],
    frac: f64,
}

impl Default for Resampler {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

impl Resampler {
    pub fn new() -> Self {
        Self { hist: [[0.0; 2]; 4], frac: 0.0 }
    }

    /// `step` = input rate / output rate. Appends interleaved L,R samples to `out`.
    pub fn process(&mut self, input: &[[f32; 2]], step: f64, out: &mut Vec<f32>) {
        let step = step.max(1e-3);
        for &x in input {
            self.hist = [self.hist[1], self.hist[2], self.hist[3], x];
            while self.frac < 1.0 {
                let t = self.frac as f32;
                let h = &self.hist;
                out.push(hermite(h[0][0], h[1][0], h[2][0], h[3][0], t));
                out.push(hermite(h[0][1], h[1][1], h[2][1], h[3][1], t));
                self.frac += step;
            }
            self.frac -= 1.0;
        }
    }
}

/// FIFO between the DSP thread and the sound-card callback (interleaved stereo f32).
pub struct AudioOut {
    buffer: Mutex<VecDeque<f32>>,
    /// Output device sample rate; 0 = no device.
    rate: AtomicU32,
    volume_bits: AtomicU32,
    muted: AtomicBool,
    primed: AtomicBool,
}

impl Default for AudioOut {
    fn default() -> Self {
        Self::new()
    }
}

/// Buffer fill the drift controller aims for, seconds.
pub const TARGET_FILL: f32 = 0.15;
const MAX_FILL: f32 = 0.6;

impl AudioOut {
    pub fn new() -> Self {
        Self {
            buffer: Mutex::new(VecDeque::new()),
            rate: AtomicU32::new(0),
            volume_bits: AtomicU32::new(0.7f32.to_bits()),
            muted: AtomicBool::new(false),
            primed: AtomicBool::new(false),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }
    pub fn set_rate(&self, hz: u32) {
        self.rate.store(hz, Ordering::Relaxed);
        self.clear();
    }
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }
    pub fn set_volume(&self, v: f32) {
        self.volume_bits.store(v.clamp(0.0, 1.5).to_bits(), Ordering::Relaxed);
    }
    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }
    pub fn set_muted(&self, m: bool) {
        self.muted.store(m, Ordering::Relaxed);
        if m {
            self.clear();
        }
    }
    /// True when audio should be produced at all.
    pub fn active(&self) -> bool {
        self.rate() > 0 && !self.muted()
    }

    pub fn clear(&self) {
        self.buffer.lock().unwrap().clear();
        self.primed.store(false, Ordering::Relaxed);
    }

    /// Buffered audio in seconds.
    pub fn fill_secs(&self) -> f32 {
        let r = self.rate();
        if r == 0 {
            return 0.0;
        }
        self.buffer.lock().unwrap().len() as f32 / (2.0 * r as f32)
    }

    /// Called by the DSP thread with interleaved L,R samples.
    pub fn push(&self, samples: &[f32]) {
        let r = self.rate();
        if r == 0 || self.muted() {
            return;
        }
        let mut b = self.buffer.lock().unwrap();
        b.extend(samples.iter().copied());
        let max = (MAX_FILL * 2.0 * r as f32) as usize & !1;
        if b.len() > max {
            let drop = (b.len() - max + 1) & !1;
            b.drain(..drop);
        }
    }

    /// Called by the sound-card callback: fills `out` with `channels`-interleaved frames.
    pub fn pull(&self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let vol = if self.muted() { 0.0 } else { self.volume() };
        let mut b = self.buffer.lock().unwrap();
        let need = (TARGET_FILL * 2.0 * self.rate() as f32) as usize;
        if !self.primed.load(Ordering::Relaxed) {
            if b.len() >= need {
                self.primed.store(true, Ordering::Relaxed);
            } else {
                out.iter_mut().for_each(|s| *s = 0.0);
                return;
            }
        }
        for frame in out.chunks_mut(channels) {
            let (l, r) = if b.len() >= 2 {
                (b.pop_front().unwrap_or(0.0), b.pop_front().unwrap_or(0.0))
            } else {
                self.primed.store(false, Ordering::Relaxed);
                (0.0, 0.0)
            };
            let (l, r) = ((l * vol).clamp(-1.0, 1.0), (r * vol).clamp(-1.0, 1.0));
            match frame.len() {
                1 => frame[0] = 0.5 * (l + r),
                _ => {
                    frame[0] = l;
                    frame[1] = r;
                    for s in frame.iter_mut().skip(2) {
                        *s = 0.0;
                    }
                }
            }
        }
    }
}

/// Drift controller: resampling step that keeps the FIFO near `TARGET_FILL`
/// despite the SDR and sound-card clocks differing slightly.
pub fn resample_step(out_rate: u32, fill_secs: f32) -> f64 {
    let err = ((fill_secs - TARGET_FILL) / TARGET_FILL).clamp(-1.0, 1.0) as f64;
    AUDIO_RATE as f64 / out_rate as f64 * (1.0 + 0.005 * err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_rate() {
        let mut r = Resampler::new();
        let input = vec![[0.5f32, -0.5]; 57_000];
        let mut out = Vec::new();
        r.process(&input, 57_000.0 / 48_000.0, &mut out);
        let frames = out.len() / 2;
        assert!((frames as i64 - 48_000).abs() <= 2, "{frames}");
        assert!((out[out.len() - 2] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn fifo_primes_and_pulls() {
        let a = AudioOut::new();
        a.set_rate(1000);
        let mut out = vec![1.0f32; 20];
        a.push(&[0.25; 100]); // 50 frames < 150 target
        a.pull(&mut out, 2);
        assert!(out.iter().all(|&s| s == 0.0));
        a.push(&[0.25; 300]);
        a.pull(&mut out, 2);
        assert!((out[0] - 0.25 * 0.7).abs() < 1e-6);
    }
}
