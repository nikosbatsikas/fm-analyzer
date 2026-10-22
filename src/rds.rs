//! RDS (IEC 62106) demodulation and group decoding.
//!
//! Input: complex baseband of the 57 kHz subcarrier at 19 kHz sample rate
//! (16 samples per RDS bit). Output: an [`RdsState`] with PI, PS, PTY, RT, CT, AF...

use crate::dsp::C32;
use std::collections::BTreeSet;

pub const SAMPLES_PER_BIT: usize = 16;

/// Generator polynomial x^10 + x^8 + x^7 + x^5 + x^4 + x^3 + 1.
const POLY: u32 = 0x5B9;
pub const OFFSET_A: u32 = 0x0FC;
pub const OFFSET_B: u32 = 0x198;
pub const OFFSET_C: u32 = 0x168;
pub const OFFSET_CP: u32 = 0x350;
pub const OFFSET_D: u32 = 0x1B4;
const OFFSETS: [u32; 4] = [OFFSET_A, OFFSET_B, OFFSET_C, OFFSET_D];

/// Remainder of a 26-bit block divided by the generator polynomial.
/// For an error-free block this equals the offset word.
pub fn syndrome(block: u32) -> u32 {
    let mut r = block & 0x3FF_FFFF;
    for i in (10..26).rev() {
        if r & (1 << i) != 0 {
            r ^= POLY << (i - 10);
        }
    }
    r & 0x3FF
}

/// Builds a 26-bit block (16 data bits + 10 check bits) for transmission.
pub fn make_block(data: u16, offset: u32) -> u32 {
    let d = (data as u32) << 10;
    d | (syndrome(d) ^ offset)
}

pub const PTY_NAMES: [&str; 32] = [
    "None", "News", "Current Affairs", "Information", "Sport", "Education", "Drama",
    "Culture", "Science", "Varied", "Pop Music", "Rock Music", "Easy Listening",
    "Light Classical", "Serious Classical", "Other Music", "Weather", "Finance",
    "Children's", "Social Affairs", "Religion", "Phone-In", "Travel", "Leisure",
    "Jazz Music", "Country Music", "National Music", "Oldies Music", "Folk Music",
    "Documentary", "Alarm Test", "Alarm",
];

/// Decoded RDS information for the current station.
#[derive(Clone, Debug, Default)]
pub struct RdsState {
    pub synced: bool,
    pub pi: Option<u16>,
    pub pty: Option<u8>,
    pub tp: bool,
    pub ta: bool,
    pub music: bool,
    pub ps: [u8; 8],
    pub ps_seen: u8,
    pub rt: Vec<u8>,
    pub rt_ab: Option<bool>,
    pub clock: Option<String>,
    /// Alternative frequencies in units of 100 kHz (e.g. 1015 = 101.5 MHz).
    pub af: BTreeSet<u16>,
    /// Group counters: [type][version A=0 / B=1].
    pub groups: [[u32; 2]; 16],
    pub blocks_total: u64,
    pub blocks_bad: u64,
}

impl RdsState {
    pub fn new() -> Self {
        Self {
            ps: [b' '; 8],
            rt: vec![b' '; 64],
            ..Default::default()
        }
    }

    pub fn ps_string(&self) -> String {
        self.ps.iter().map(|&c| rds_char(c)).collect()
    }

    pub fn rt_string(&self) -> String {
        let end = self.rt.iter().position(|&c| c == 0x0D).unwrap_or(self.rt.len());
        self.rt[..end]
            .iter()
            .map(|&c| rds_char(c))
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    pub fn pty_name(&self) -> &'static str {
        self.pty.map(|p| PTY_NAMES[p as usize & 31]).unwrap_or("-")
    }

    /// Block error rate in percent since the last reset.
    pub fn bler(&self) -> f32 {
        if self.blocks_total == 0 {
            0.0
        } else {
            100.0 * self.blocks_bad as f32 / self.blocks_total as f32
        }
    }

    pub fn total_groups(&self) -> u32 {
        self.groups.iter().map(|g| g[0] + g[1]).sum()
    }
}

/// Maps the RDS basic character set (IEC 62106 Annex E) to Unicode.
/// Printable ASCII is shared with the RDS table; other codes are shown as '·'
/// (extend this table if you need national characters).
pub fn rds_char(c: u8) -> char {
    match c {
        0x24 => '¤',
        0x5E => '―',
        0x60 => '║',
        0x7E => '¯',
        0x20..=0x7D => c as char,
        0x80 => 'á',
        0x81 => 'à',
        0x82 => 'é',
        0x83 => 'è',
        0x84 => 'í',
        0x85 => 'ì',
        0x86 => 'ó',
        0x87 => 'ò',
        0x88 => 'ú',
        0x89 => 'ù',
        0x8D => 'ß',
        0x91 => 'ä',
        0x97 => 'ö',
        0x99 => 'ü',
        _ => '·',
    }
}

/// Block synchroniser and group decoder working on the differentially decoded bit stream.
pub struct RdsDecoder {
    reg: u32,
    bit_count: u64,
    synced: bool,
    last_hit: Option<(u64, usize)>,
    block_bits: u32,
    expected: usize,
    group: [Option<u16>; 4],
    bad_run: u32,
    pub state: RdsState,
}

impl Default for RdsDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl RdsDecoder {
    pub fn new() -> Self {
        Self {
            reg: 0,
            bit_count: 0,
            synced: false,
            last_hit: None,
            block_bits: 0,
            expected: 0,
            group: [None; 4],
            bad_run: 0,
            state: RdsState::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    fn offset_index(r: u32) -> Option<usize> {
        match r {
            OFFSET_A => Some(0),
            OFFSET_B => Some(1),
            OFFSET_C | OFFSET_CP => Some(2),
            OFFSET_D => Some(3),
            _ => None,
        }
    }

    pub fn push_bit(&mut self, bit: bool) {
        self.reg = ((self.reg << 1) | bit as u32) & 0x3FF_FFFF;
        self.bit_count += 1;

        if !self.synced {
            if self.bit_count < 26 {
                return;
            }
            if let Some(idx) = Self::offset_index(syndrome(self.reg)) {
                if let Some((pos, pidx)) = self.last_hit {
                    let d = self.bit_count - pos;
                    if d % 26 == 0 && d / 26 <= 6 && (pidx + (d / 26) as usize) % 4 == idx {
                        self.synced = true;
                        self.state.synced = true;
                        self.block_bits = 0;
                        self.bad_run = 0;
                        self.group = [None; 4];
                        if idx < 3 {
                            self.group[idx] = Some((self.reg >> 10) as u16);
                        }
                        self.expected = (idx + 1) % 4;
                        return;
                    }
                }
                self.last_hit = Some((self.bit_count, idx));
            }
            return;
        }

        self.block_bits += 1;
        if self.block_bits < 26 {
            return;
        }
        self.block_bits = 0;

        let r = syndrome(self.reg);
        let ok = if self.expected == 2 {
            r == OFFSET_C || r == OFFSET_CP
        } else {
            r == OFFSETS[self.expected]
        };
        self.state.blocks_total += 1;
        if ok {
            self.group[self.expected] = Some((self.reg >> 10) as u16);
            self.bad_run = 0;
        } else {
            self.state.blocks_bad += 1;
            self.group[self.expected] = None;
            self.bad_run += 1;
            if self.bad_run >= 12 {
                self.synced = false;
                self.state.synced = false;
                self.last_hit = None;
                return;
            }
        }
        if self.expected == 3 {
            let g = self.group;
            self.decode_group(g);
            self.group = [None; 4];
        }
        self.expected = (self.expected + 1) % 4;
    }

    fn decode_group(&mut self, g: [Option<u16>; 4]) {
        let st = &mut self.state;
        if let Some(a) = g[0] {
            st.pi = Some(a);
        }
        let Some(b) = g[1] else { return };
        let gtype = (b >> 12) as usize;
        let ver_b = (b >> 11) & 1 == 1;
        if ver_b {
            if let Some(c) = g[2] {
                st.pi = Some(c); // block C' repeats PI
            }
        }
        st.groups[gtype][ver_b as usize] += 1;
        st.tp = (b >> 10) & 1 == 1;
        st.pty = Some(((b >> 5) & 31) as u8);

        match gtype {
            0 => {
                st.ta = (b >> 4) & 1 == 1;
                st.music = (b >> 3) & 1 == 1;
                let seg = (b & 3) as usize;
                if let Some(d) = g[3] {
                    st.ps[seg * 2] = (d >> 8) as u8;
                    st.ps[seg * 2 + 1] = d as u8;
                    st.ps_seen |= 1 << seg;
                }
                if !ver_b {
                    if let Some(c) = g[2] {
                        for code in [(c >> 8) as u8, c as u8] {
                            if (1..=204).contains(&code) {
                                st.af.insert(875 + code as u16);
                            }
                        }
                    }
                }
            }
            2 => {
                let ab = (b >> 4) & 1 == 1;
                if st.rt_ab.is_some() && st.rt_ab != Some(ab) {
                    st.rt = vec![b' '; 64];
                }
                st.rt_ab = Some(ab);
                let seg = (b & 15) as usize;
                if !ver_b {
                    if let (Some(c), Some(d)) = (g[2], g[3]) {
                        let i = seg * 4;
                        st.rt[i] = (c >> 8) as u8;
                        st.rt[i + 1] = c as u8;
                        st.rt[i + 2] = (d >> 8) as u8;
                        st.rt[i + 3] = d as u8;
                    }
                } else if let Some(d) = g[3] {
                    let i = seg * 2;
                    st.rt[i] = (d >> 8) as u8;
                    st.rt[i + 1] = d as u8;
                }
            }
            4 if !ver_b => {
                if let (Some(c), Some(d)) = (g[2], g[3]) {
                    let mjd = (((b & 3) as u32) << 15) | ((c as u32) >> 1);
                    let hour = (((c & 1) as u32) << 4) | ((d as u32) >> 12);
                    let minute = ((d >> 6) & 63) as u32;
                    let neg = (d >> 5) & 1 == 1;
                    let half_hours = (d & 31) as i32;
                    if hour < 24 && minute < 60 {
                        let off_min = if neg { -half_hours * 30 } else { half_hours * 30 };
                        let (y, m, dd) = mjd_to_date(mjd);
                        st.clock = Some(format!(
                            "{y:04}-{m:02}-{dd:02} {hour:02}:{minute:02} UTC ({}{:02}:{:02})",
                            if neg { '-' } else { '+' },
                            off_min.abs() / 60,
                            off_min.abs() % 60
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Modified Julian Date to (year, month, day), per IEC 62106 Annex G.
pub fn mjd_to_date(mjd: u32) -> (i32, u32, u32) {
    let mjd = mjd as f64;
    let yp = ((mjd - 15078.2) / 365.25).floor();
    let mp = ((mjd - 14956.1 - (yp * 365.25).floor()) / 30.6001).floor();
    let d = mjd - 14956.0 - (yp * 365.25).floor() - (mp * 30.6001).floor();
    let k = if mp == 14.0 || mp == 15.0 { 1.0 } else { 0.0 };
    let y = yp + k + 1900.0;
    let m = mp - 1.0 - k * 12.0;
    (y as i32, m as u32, d as u32)
}

/// BPSK/biphase demodulator: phase alignment, symbol timing and differential decoding.
pub struct RdsDemod {
    z2_avg: C32,
    rot: C32,
    hist: [f32; SAMPLES_PER_BIT],
    hpos: usize,
    energy: [f32; SAMPLES_PER_BIT],
    idx: usize,
    best: usize,
    prev_sym: bool,
    /// Mean power of the 57 kHz baseband (input units squared).
    pub power: f32,
    pub decoder: RdsDecoder,
}

impl Default for RdsDemod {
    fn default() -> Self {
        Self::new()
    }
}

impl RdsDemod {
    pub fn new() -> Self {
        Self {
            z2_avg: C32::default(),
            rot: C32::new(1.0, 0.0),
            hist: [0.0; SAMPLES_PER_BIT],
            hpos: 0,
            energy: [0.0; SAMPLES_PER_BIT],
            idx: 0,
            best: 0,
            prev_sym: false,
            power: 0.0,
            decoder: RdsDecoder::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn push(&mut self, z: C32) {
        self.power += 0.0005 * (z.norm_sqr() - self.power);

        // BPSK carrier phase from the squared signal; keep the rotation continuous.
        self.z2_avg = self.z2_avg.scale(0.999) + (z * z).scale(0.001);
        let r1 = C32::from_polar(1.0, -self.z2_avg.arg() / 2.0);
        let r2 = r1.scale(-1.0);
        self.rot = if (r1 - self.rot).norm_sqr() < (r2 - self.rot).norm_sqr() {
            r1
        } else {
            r2
        };
        let x = (z * self.rot).re;

        self.hist[self.hpos] = x;
        self.hpos = (self.hpos + 1) % SAMPLES_PER_BIT;

        // Biphase matched filter over the last bit period (oldest sample first).
        let half = SAMPLES_PER_BIT / 2;
        let mut corr = 0.0;
        for k in 0..SAMPLES_PER_BIT {
            let v = self.hist[(self.hpos + k) % SAMPLES_PER_BIT];
            if k < half {
                corr += v;
            } else {
                corr -= v;
            }
        }

        self.energy[self.idx] = self.energy[self.idx] * 0.97 + corr.abs() * 0.03;
        if self.idx == self.best {
            let sym = corr > 0.0;
            self.decoder.push_bit(sym != self.prev_sym);
            self.prev_sym = sym;
        }
        self.idx += 1;
        if self.idx == SAMPLES_PER_BIT {
            self.idx = 0;
            let (arg, e) = self
                .energy
                .iter()
                .enumerate()
                .fold((0, 0.0f32), |acc, (i, &e)| if e > acc.1 { (i, e) } else { acc });
            if e > self.energy[self.best] * 1.15 {
                self.best = arg;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkwords_roundtrip() {
        for (off, idx) in [(OFFSET_A, 0), (OFFSET_B, 1), (OFFSET_C, 2), (OFFSET_D, 3)] {
            let b = make_block(0xBEEF, off);
            assert_eq!(syndrome(b), off);
            assert_eq!(RdsDecoder::offset_index(syndrome(b)), Some(idx));
        }
        // a single bit error must break the syndrome
        let b = make_block(0x1234, OFFSET_A) ^ (1 << 17);
        assert_ne!(syndrome(b), OFFSET_A);
    }

    #[test]
    fn mjd_example() {
        // IEC 62106 example: MJD 45218 = 1982-09-06
        assert_eq!(mjd_to_date(45218), (1982, 9, 6));
    }
}
