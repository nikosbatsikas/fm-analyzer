//! FM Analyzer core library: DSP, FM/MPX measurements, RDS decoding and I/Q sources.
//!
//! Everything except the optional `usb` source is dependency-free, so the
//! signal-processing core can be tested and reused without the GUI.

pub mod audio;
pub mod dsp;
pub mod fm;
pub mod rds;
pub mod source;
pub mod worker;

#[cfg(test)]
mod testsignal;

#[cfg(test)]
mod tests {
    use crate::fm::{Analyzer, Snapshot, IQ_RATE};
    use crate::testsignal::*;
    use crate::source::IqSource;

    fn run_demo(seconds: f32, center: u32) -> (Snapshot, Analyzer) {
        let mut src = TestSignal::new(false);
        src.set_center_freq(center).unwrap();
        let mut an = Analyzer::new();
        let mut buf = vec![0u8; 65_536];
        let total = (IQ_RATE as f32 * seconds) as usize * 2;
        let mut done = 0;
        let mut last = None;
        while done < total {
            let n = src.read(&mut buf).unwrap();
            if let Some(s) = an.process(&buf[..n]).pop() {
                last = Some(s);
            }
            done += n;
        }
        (last.expect("no snapshot"), an)
    }

    #[test]
    fn test_signal_station_measurements_and_rds() {
        let (s, _) = run_demo(6.0, TEST_FREQ_HZ);
        println!(
            "peak {:.1} kHz, pilot {:.2} kHz locked={} ({:.2} Hz), rds {:.2} kHz, mpx {:.2} dBr, ch {:.1} dBFS",
            s.peak_khz, s.pilot_khz, s.pilot_locked, s.pilot_freq_hz, s.rds_khz, s.mpx_power_60s, s.channel_dbfs
        );
        println!(
            "PI {:04X?} PS '{}' PTY {} RT '{}' CT {:?} AF {:?} BLER {:.1}% groups {}",
            s.rds.pi, s.rds.ps_string(), s.rds.pty_name(), s.rds.rt_string(), s.rds.clock,
            s.rds.af, s.rds.bler(), s.rds.total_groups()
        );
        assert!(s.pilot_locked, "pilot not locked");
        assert!((s.pilot_khz - TEST_PILOT_HZ as f32 / 1000.0).abs() < 0.4);
        assert!((s.pilot_freq_hz - 19_000.0).abs() < 2.0);
        assert!(s.peak_khz > 45.0 && s.peak_khz < 60.0, "peak {}", s.peak_khz);
        assert!((s.rds_khz - TEST_RDS_HZ as f32 / 1000.0).abs() < 0.6, "rds {}", s.rds_khz);
        assert_eq!(s.rds.pi, Some(TEST_PI));
        assert_eq!(s.rds.ps_string(), std::str::from_utf8(TEST_PS).unwrap());
        assert_eq!(s.rds.pty, Some(10));
        assert!(s.rds.tp);
        assert!(s.rds.af.contains(&1015));
        assert!(s.rds.bler() < 5.0);
    }

    #[test]
    fn test_signal_radiotext_and_clock() {
        let (s, _) = run_demo(16.0, TEST_FREQ_HZ);
        assert_eq!(s.rds.rt_string(), TEST_RT);
        let ct = s.rds.clock.clone().expect("no CT");
        assert!(ct.starts_with("2026-10-22 18:30"), "{ct}");
        assert!(s.mpx_seconds >= 15);
    }

    #[test]
    fn off_station_has_no_rds() {
        let (s, _) = run_demo(2.0, TEST_FREQ_HZ + 1_500_000);
        assert!(!s.pilot_locked);
        assert!(s.rds.pi.is_none());
    }

    /// Correlates `x` with a tone; returns its amplitude.
    fn tone_amp(x: &[f32], freq: f32, fs: f32) -> f32 {
        let (mut i, mut q) = (0.0f64, 0.0f64);
        for (n, &v) in x.iter().enumerate() {
            let ph = std::f64::consts::TAU * freq as f64 * n as f64 / fs as f64;
            i += v as f64 * ph.cos();
            q += v as f64 * ph.sin();
        }
        (2.0 * (i * i + q * q).sqrt() / x.len() as f64) as f32
    }

    fn decode_audio(mono: bool) -> (Vec<f32>, Vec<f32>) {
        let mut src = TestSignal::new(false);
        let mut an = Analyzer::new();
        an.set_force_mono(mono);
        let mut buf = vec![0u8; 65_536];
        let mut frames = Vec::new();
        let total = IQ_RATE as usize * 2 * 3;
        let mut done = 0;
        while done < total {
            let n = src.read(&mut buf).unwrap();
            an.process(&buf[..n]);
            an.drain_audio(&mut frames);
            done += n;
        }
        // skip the first 1.5 s (PLL lock), use 1 s (an integer number of cycles)
        let fs = crate::audio::AUDIO_RATE as usize;
        let seg = &frames[fs * 3 / 2..fs * 5 / 2];
        (seg.iter().map(|f| f[0]).collect(), seg.iter().map(|f| f[1]).collect())
    }

    #[test]
    fn stereo_decoder_separates_channels() {
        let fs = crate::audio::AUDIO_RATE;
        let (l, r) = decode_audio(false);
        let l1k = tone_amp(&l, 1000.0, fs);
        let r1k = tone_amp(&r, 1000.0, fs);
        let r400 = tone_amp(&r, 400.0, fs);
        let l400 = tone_amp(&l, 400.0, fs);
        println!("L: 1k {l1k:.3} 400 {l400:.4} | R: 400 {r400:.3} 1k {r1k:.4}");
        // test signal: L = 1 kHz at 46 kHz dev, R = 400 Hz at 0.6x; 50 us de-emphasis
        assert!((l1k - 0.65).abs() < 0.05, "L level {l1k}");
        assert!((r400 - 0.40).abs() < 0.04, "R level {r400}");
        let sep_l = 20.0 * (l1k / r1k).log10();
        let sep_r = 20.0 * (r400 / l400).log10();
        assert!(sep_l > 30.0 && sep_r > 30.0, "separation {sep_l:.1} / {sep_r:.1} dB");
    }

    #[test]
    fn forced_mono_mixes_channels() {
        let fs = crate::audio::AUDIO_RATE;
        let (l, r) = decode_audio(true);
        let (a, b) = (tone_amp(&l, 1000.0, fs), tone_amp(&r, 1000.0, fs));
        assert!((a - b).abs() < 0.01 && a > 0.2, "{a} {b}");
    }
}
