//! Small, dependency-free DSP building blocks: complex numbers, FIR design,
//! decimating FIR filters, radix-2 FFT, spectrum averaging and the 19 kHz pilot PLL.

use std::f32::consts::PI;
use std::ops::{Add, Mul, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct C32 {
    pub re: f32,
    pub im: f32,
}

impl C32 {
    pub const fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }
    #[inline]
    pub fn conj(self) -> Self {
        Self::new(self.re, -self.im)
    }
    #[inline]
    pub fn norm_sqr(self) -> f32 {
        self.re * self.re + self.im * self.im
    }
    #[inline]
    pub fn abs(self) -> f32 {
        self.norm_sqr().sqrt()
    }
    #[inline]
    pub fn arg(self) -> f32 {
        self.im.atan2(self.re)
    }
    #[inline]
    pub fn scale(self, k: f32) -> Self {
        Self::new(self.re * k, self.im * k)
    }
    #[inline]
    pub fn from_polar(r: f32, theta: f32) -> Self {
        let (s, c) = theta.sin_cos();
        Self::new(r * c, r * s)
    }
}

impl Add for C32 {
    type Output = C32;
    #[inline]
    fn add(self, o: C32) -> C32 {
        C32::new(self.re + o.re, self.im + o.im)
    }
}
impl Sub for C32 {
    type Output = C32;
    #[inline]
    fn sub(self, o: C32) -> C32 {
        C32::new(self.re - o.re, self.im - o.im)
    }
}
impl Mul for C32 {
    type Output = C32;
    #[inline]
    fn mul(self, o: C32) -> C32 {
        C32::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
}

/// Windowed-sinc (Blackman) low-pass. `cutoff` is normalised to the sample rate (0..0.5).
/// Taps are normalised for unity DC gain.
pub fn lowpass_taps(num: usize, cutoff: f32) -> Vec<f32> {
    assert!(num >= 3);
    let m = (num - 1) as f32;
    let mut h: Vec<f32> = (0..num)
        .map(|n| {
            let x = n as f32 - m / 2.0;
            let s = if x == 0.0 {
                2.0 * cutoff
            } else {
                (2.0 * PI * cutoff * x).sin() / (PI * x)
            };
            let t = n as f32 / m;
            let w = 0.42 - 0.5 * (2.0 * PI * t).cos() + 0.08 * (4.0 * PI * t).cos();
            s * w
        })
        .collect();
    let sum: f32 = h.iter().sum();
    for v in &mut h {
        *v /= sum;
    }
    h
}

/// Complex decimating FIR. Only every `decim`-th output is computed.
pub struct DecimFirC {
    taps: Vec<f32>,
    bi: Vec<f32>,
    bq: Vec<f32>,
    pos: usize,
    decim: usize,
    phase: usize,
}

impl DecimFirC {
    pub fn new(taps: Vec<f32>, decim: usize) -> Self {
        let n = taps.len();
        Self {
            taps,
            bi: vec![0.0; 2 * n],
            bq: vec![0.0; 2 * n],
            pos: 0,
            decim: decim.max(1),
            phase: 0,
        }
    }

    #[inline]
    pub fn push(&mut self, i: f32, q: f32) -> Option<C32> {
        let n = self.taps.len();
        self.bi[self.pos] = i;
        self.bi[self.pos + n] = i;
        self.bq[self.pos] = q;
        self.bq[self.pos + n] = q;
        self.pos += 1;
        if self.pos == n {
            self.pos = 0;
        }
        self.phase += 1;
        if self.phase < self.decim {
            return None;
        }
        self.phase = 0;
        let si = &self.bi[self.pos..self.pos + n];
        let sq = &self.bq[self.pos..self.pos + n];
        let (mut ai, mut aq) = (0.0f32, 0.0f32);
        for ((t, x), y) in self.taps.iter().zip(si).zip(sq) {
            ai += t * x;
            aq += t * y;
        }
        Some(C32::new(ai, aq))
    }
}

/// Iterative radix-2 complex FFT.
pub struct Fft {
    n: usize,
    rev: Vec<usize>,
    tw: Vec<C32>,
}

impl Fft {
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 2, "FFT size must be a power of two");
        let bits = n.trailing_zeros();
        let rev = (0..n)
            .map(|i| i.reverse_bits() >> (usize::BITS - bits))
            .collect();
        let tw = (0..n / 2)
            .map(|k| C32::from_polar(1.0, -2.0 * PI * k as f32 / n as f32))
            .collect();
        Self { n, rev, tw }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn process(&self, buf: &mut [C32]) {
        let n = self.n;
        assert_eq!(buf.len(), n);
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                buf.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let w = self.tw[k * step];
                    let a = buf[start + k];
                    let b = buf[start + k + half] * w;
                    buf[start + k] = a + b;
                    buf[start + k + half] = a - b;
                }
            }
            len <<= 1;
        }
    }
}

/// Windowed, exponentially averaged power spectrum.
/// Stored values are |X|^2 / (sum w)^2, i.e. a complex tone of amplitude A gives A^2.
pub struct Spectrum {
    fft: Fft,
    window: Vec<f32>,
    wsum: f32,
    avg: Vec<f32>,
    work: Vec<C32>,
    pub alpha: f32,
    first: bool,
}

impl Spectrum {
    pub fn new(n: usize, alpha: f32) -> Self {
        let window: Vec<f32> = (0..n)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
            .collect();
        let wsum = window.iter().sum();
        Self {
            fft: Fft::new(n),
            window,
            wsum,
            avg: vec![0.0; n],
            work: vec![C32::default(); n],
            alpha,
            first: true,
        }
    }

    pub fn len(&self) -> usize {
        self.fft.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fft.is_empty()
    }

    /// `samples` must be in chronological order and exactly `len()` long.
    pub fn update<I: Iterator<Item = C32>>(&mut self, samples: I) {
        for (k, s) in samples.take(self.work.len()).enumerate() {
            self.work[k] = s.scale(self.window[k]);
        }
        self.fft.process(&mut self.work);
        let norm = 1.0 / (self.wsum * self.wsum);
        for (a, x) in self.avg.iter_mut().zip(&self.work) {
            let p = x.norm_sqr() * norm;
            if self.first {
                *a = p;
            } else {
                *a += self.alpha * (p - *a);
            }
        }
        self.first = false;
    }

    pub fn power(&self) -> &[f32] {
        &self.avg
    }

    pub fn reset(&mut self) {
        self.first = true;
    }
}

/// Second-order PLL locking to the 19 kHz stereo pilot in the MPX signal.
/// After lock the pilot is approximately `amp * cos(phase)`.
///
/// Loop state is kept in f64: the integrator step (~1e-8 rad/sample) is below
/// f32 resolution at the nominal 0.52 rad/sample and would never pull in.
pub struct PilotPll {
    pub phase: f32,
    phase64: f64,
    freq_off: f64,
    nominal: f64,
    i_lp: f32,
    q_lp: f32,
    alpha: f32,
    kp: f64,
    ki: f64,
    /// Pilot amplitude, in the same units as the input (Hz of deviation).
    pub amp: f32,
    lock_lp: f32,
}

impl PilotPll {
    pub fn new(fs: f32, f0: f32) -> Self {
        let fs = fs as f64;
        let nominal = std::f64::consts::TAU * f0 as f64 / fs;
        let wn = std::f64::consts::TAU * 8.0 / fs; // loop natural frequency ~8 Hz
        let zeta = 0.707;
        Self {
            phase: 0.0,
            phase64: 0.0,
            freq_off: 0.0,
            nominal,
            i_lp: 0.0,
            q_lp: 0.0,
            alpha: 1.0 - (-2.0 * PI * 80.0 / fs as f32).exp(),
            kp: 2.0 * zeta * wn,
            ki: wn * wn,
            amp: 0.0,
            lock_lp: 0.0,
        }
    }

    #[inline]
    pub fn step(&mut self, x: f32) {
        let (s, c) = self.phase.sin_cos();
        self.i_lp += self.alpha * (x * c - self.i_lp);
        self.q_lp += self.alpha * (-x * s - self.q_lp);
        let err = self.q_lp.atan2(self.i_lp);
        let e = err as f64;
        self.freq_off += self.ki * e;
        let lim = self.nominal * 0.002; // +-38 Hz pull range
        self.freq_off = self.freq_off.clamp(-lim, lim);
        self.phase64 += self.nominal + self.freq_off + self.kp * e;
        if self.phase64 >= std::f64::consts::TAU {
            self.phase64 -= std::f64::consts::TAU;
        } else if self.phase64 < 0.0 {
            self.phase64 += std::f64::consts::TAU;
        }
        self.phase = self.phase64 as f32;
        self.amp = 2.0 * (self.i_lp * self.i_lp + self.q_lp * self.q_lp).sqrt();
        self.lock_lp += 0.0005 * (err.cos() - self.lock_lp);
    }

    /// True when the loop is phase-locked to a pilot of reasonable level.
    pub fn locked(&self) -> bool {
        self.lock_lp > 0.9 && self.amp > 1000.0
    }

    pub fn freq_hz(&self, fs: f32) -> f32 {
        ((self.nominal + self.freq_off) * fs as f64 / std::f64::consts::TAU) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_finds_tone() {
        let n = 256;
        let fft = Fft::new(n);
        let mut buf: Vec<C32> = (0..n)
            .map(|i| C32::from_polar(1.0, 2.0 * PI * 10.0 * i as f32 / n as f32))
            .collect();
        fft.process(&mut buf);
        let (best, _) = buf
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
            .unwrap();
        assert_eq!(best, 10);
        assert!((buf[10].abs() - n as f32).abs() < 0.01 * n as f32);
    }

    #[test]
    fn lowpass_unity_dc() {
        let h = lowpass_taps(101, 0.1);
        let s: f32 = h.iter().sum();
        assert!((s - 1.0).abs() < 1e-5);
    }
}

#[cfg(test)]
mod pll_tests {
    use super::*;

    #[test]
    fn pll_locks_to_offset_pilot() {
        let fs = 228_000.0;
        let mut p = PilotPll::new(fs, 19_000.0);
        for n in 0..(fs as usize * 2) {
            let t = n as f64 / fs as f64;
            p.step(6750.0 * (std::f64::consts::TAU * 19_003.0 * t).sin() as f32);
        }
        assert!(p.locked());
        assert!((p.freq_hz(fs) - 19_003.0).abs() < 0.2, "{}", p.freq_hz(fs));
        assert!((p.amp - 6750.0).abs() < 70.0);
    }
}
