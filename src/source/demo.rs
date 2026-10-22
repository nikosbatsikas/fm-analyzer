//! Built-in test signal: a stereo FM station at 98.0 MHz with pilot and RDS,
//! plus noise. Lets you try the app (and run the tests) without hardware.

use super::IqSource;
use crate::rds::{make_block, OFFSET_A, OFFSET_B, OFFSET_C, OFFSET_D};
use std::f64::consts::TAU;
use std::io;
use std::time::{Duration, Instant};

pub const DEMO_FREQ_HZ: u32 = 98_000_000;
pub const DEMO_PI: u16 = 0x2A3F;
pub const DEMO_PS: &[u8; 8] = b"FM ANLZR";
pub const DEMO_RT: &str = "FM Analyzer demo signal - stereo, pilot and RDS test";
pub const DEMO_PILOT_HZ: f64 = 6_750.0;
pub const DEMO_RDS_HZ: f64 = 3_500.0;

const FS: f64 = 2_280_000.0;
const SAMPLES_PER_RDS_BIT: u32 = 1920; // 2.28 MHz / 1187.5 bit/s

/// Generates the RDS group sequence and turns it into a bit stream.
pub struct RdsEncoder {
    groups_sent: u32,
    bits: Vec<bool>,
    pos: usize,
}

impl Default for RdsEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl RdsEncoder {
    pub fn new() -> Self {
        Self { groups_sent: 0, bits: Vec::new(), pos: 0 }
    }

    fn next_group(&mut self) -> [u16; 4] {
        let pty: u16 = 10; // Pop Music
        let tp: u16 = 1;
        let n = self.groups_sent;
        self.groups_sent += 1;
        let slot = n % 10;
        let mut rt = [b' '; 64];
        for (i, b) in DEMO_RT.bytes().take(63).enumerate() {
            rt[i] = b;
        }
        rt[DEMO_RT.len().min(63)] = 0x0D;
        if n % 40 == 39 {
            // 4A clock-time: 2026-10-22 18:30 UTC, +03:00
            let mjd: u32 = 61335;
            let (hour, min, off) = (18u32, 30u32, 6u32);
            let b = (4 << 12) | (tp << 10) | (pty << 5) | ((mjd >> 15) & 3) as u16;
            let c = (((mjd & 0x7FFF) << 1) | (hour >> 4)) as u16;
            let d = (((hour & 15) << 12) | (min << 6) | off) as u16;
            return [DEMO_PI, b, c, d];
        }
        if slot < 6 {
            // 0A: PS (4 segments), AF list
            let seg = (n / 10 * 6 + slot) as u16 % 4;
            let b = (tp << 10) | (pty << 5) | (1 << 3) | seg;
            let c = (225u16 << 8) | (1015 - 875); // 1 AF: 101.5 MHz
            let d = ((DEMO_PS[seg as usize * 2] as u16) << 8) | DEMO_PS[seg as usize * 2 + 1] as u16;
            [DEMO_PI, b, c, d]
        } else {
            // 2A: radiotext, 16 segments
            let seg = ((n / 10) * 4 + (slot - 6)) as usize % 16;
            let b = (2 << 12) | (tp << 10) | (pty << 5) | seg as u16;
            let c = ((rt[seg * 4] as u16) << 8) | rt[seg * 4 + 1] as u16;
            let d = ((rt[seg * 4 + 2] as u16) << 8) | rt[seg * 4 + 3] as u16;
            [DEMO_PI, b, c, d]
        }
    }

    /// Next data bit (before differential encoding).
    pub fn next_bit(&mut self) -> bool {
        if self.pos >= self.bits.len() {
            let g = self.next_group();
            self.bits.clear();
            for (data, off) in g.iter().zip([OFFSET_A, OFFSET_B, OFFSET_C, OFFSET_D]) {
                let blk = make_block(*data, off);
                for i in (0..26).rev() {
                    self.bits.push((blk >> i) & 1 == 1);
                }
            }
            self.pos = 0;
        }
        self.pos += 1;
        self.bits[self.pos - 1]
    }
}

pub struct DemoSource {
    realtime: bool,
    started: Instant,
    produced: u64,
    center: u32,
    gain_db: f32,
    // oscillators (cycles, wrapped to [0,1))
    pilot: f64,
    tone_l: f64,
    tone_r: f64,
    rf_phase: f64,
    // rds
    enc: RdsEncoder,
    rds_count: u32,
    rds_level: bool,
    rds_shaped: f64,
    rng: u64,
}

impl DemoSource {
    /// `realtime = true` paces output at 2.28 MS/s like real hardware.
    pub fn new(realtime: bool) -> Self {
        Self {
            realtime,
            started: Instant::now(),
            produced: 0,
            center: DEMO_FREQ_HZ,
            gain_db: 30.0,
            pilot: 0.0,
            tone_l: 0.0,
            tone_r: 0.0,
            rf_phase: 0.0,
            enc: RdsEncoder::new(),
            rds_count: 0,
            rds_level: false,
            rds_shaped: 0.0,
            rng: 0x1234_5678_9ABC_DEF1,
        }
    }

    #[inline]
    fn noise(&mut self) -> f64 {
        // sum of 4 uniforms ~ gaussian, unit variance-ish
        let mut s = 0.0;
        for _ in 0..4 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 7;
            self.rng ^= self.rng << 17;
            s += (self.rng >> 11) as f64 / (1u64 << 53) as f64 - 0.5;
        }
        s * 1.7
    }

    fn mpx_sample(&mut self) -> f64 {
        // audio: L = 1 kHz, R = 400 Hz
        let l = (TAU * self.tone_l).sin();
        let r = 0.6 * (TAU * self.tone_r).sin();
        let m = (l + r) * 0.5;
        let s = (l - r) * 0.5;
        let pp = TAU * self.pilot;

        // RDS: differential + biphase coding, shaped with a 1-pole low-pass
        if self.rds_count == 0 {
            let bit = self.enc.next_bit();
            self.rds_level ^= bit;
        }
        let first_half = self.rds_count < SAMPLES_PER_RDS_BIT / 2;
        let sym = if self.rds_level == first_half { 1.0 } else { -1.0 };
        self.rds_shaped += 0.0066 * (sym - self.rds_shaped);
        self.rds_count = (self.rds_count + 1) % SAMPLES_PER_RDS_BIT;

        let mpx = 46_000.0 * m
            + 46_000.0 * s * (2.0 * pp).sin()
            + DEMO_PILOT_HZ * pp.sin()
            + DEMO_RDS_HZ * self.rds_shaped * (3.0 * pp).sin();

        self.tone_l = (self.tone_l + 1000.0 / FS).fract();
        self.tone_r = (self.tone_r + 400.0 / FS).fract();
        self.pilot = (self.pilot + 19_000.0 / FS).fract();
        mpx
    }
}

impl IqSource for DemoSource {
    fn name(&self) -> String {
        "Demo signal (98.0 MHz)".into()
    }
    fn set_sample_rate(&mut self, _hz: u32) -> io::Result<()> {
        Ok(())
    }
    fn set_center_freq(&mut self, hz: u32) -> io::Result<()> {
        self.center = hz;
        Ok(())
    }
    fn set_gain(&mut self, tenth_db: Option<i32>) -> io::Result<()> {
        self.gain_db = tenth_db.map(|g| g as f32 / 10.0).unwrap_or(30.0);
        Ok(())
    }
    fn set_ppm(&mut self, _ppm: i32) -> io::Result<()> {
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len() / 2;
        let offset = DEMO_FREQ_HZ as f64 - self.center as f64;
        let visible = offset.abs() < FS / 2.0;
        let amp = 0.25 * 10f64.powf((self.gain_db as f64 - 30.0) / 40.0);
        let amp = amp.min(0.9);
        for k in 0..n {
            let mpx = self.mpx_sample();
            let (mut i, mut q) = (0.0, 0.0);
            if visible {
                self.rf_phase = (self.rf_phase + (offset + mpx) / FS).fract();
                let (s, c) = (TAU * self.rf_phase).sin_cos();
                i = amp * c;
                q = amp * s;
            }
            i += 0.012 * self.noise();
            q += 0.012 * self.noise();
            buf[2 * k] = (i * 127.5 + 127.5).round().clamp(0.0, 255.0) as u8;
            buf[2 * k + 1] = (q * 127.5 + 127.5).round().clamp(0.0, 255.0) as u8;
        }
        self.produced += n as u64;
        if self.realtime {
            let due = Duration::from_secs_f64(self.produced as f64 / FS);
            let el = self.started.elapsed();
            if due > el {
                std::thread::sleep(due - el);
            }
        }
        Ok(n * 2)
    }
}
