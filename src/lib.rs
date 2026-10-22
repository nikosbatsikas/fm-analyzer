//! FM Analyzer core library: DSP, FM/MPX measurements, RDS decoding and I/Q sources.
//!
//! Everything except the optional `usb` source is dependency-free, so the
//! signal-processing core can be tested and reused without the GUI.

pub mod dsp;
pub mod fm;
pub mod rds;
pub mod source;
pub mod worker;

#[cfg(test)]
mod tests {
    use crate::fm::{Analyzer, Snapshot, IQ_RATE};
    use crate::source::demo::*;
    use crate::source::IqSource;

    fn run_demo(seconds: f32, center: u32) -> (Snapshot, Analyzer) {
        let mut src = DemoSource::new(false);
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
    fn demo_station_measurements_and_rds() {
        let (s, _) = run_demo(6.0, DEMO_FREQ_HZ);
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
        assert!((s.pilot_khz - DEMO_PILOT_HZ as f32 / 1000.0).abs() < 0.4);
        assert!((s.pilot_freq_hz - 19_000.0).abs() < 2.0);
        assert!(s.peak_khz > 45.0 && s.peak_khz < 60.0, "peak {}", s.peak_khz);
        assert!((s.rds_khz - DEMO_RDS_HZ as f32 / 1000.0).abs() < 0.6, "rds {}", s.rds_khz);
        assert_eq!(s.rds.pi, Some(DEMO_PI));
        assert_eq!(s.rds.ps_string(), std::str::from_utf8(DEMO_PS).unwrap());
        assert_eq!(s.rds.pty, Some(10));
        assert!(s.rds.tp);
        assert!(s.rds.af.contains(&1015));
        assert!(s.rds.bler() < 5.0);
    }

    #[test]
    fn demo_radiotext_and_clock() {
        let (s, _) = run_demo(16.0, DEMO_FREQ_HZ);
        assert_eq!(s.rds.rt_string(), DEMO_RT);
        let ct = s.rds.clock.clone().expect("no CT");
        assert!(ct.starts_with("2026-10-22 18:30"), "{ct}");
        assert!(s.mpx_seconds >= 15);
    }

    #[test]
    fn off_station_has_no_rds() {
        let (s, _) = run_demo(2.0, DEMO_FREQ_HZ + 1_500_000);
        assert!(!s.pilot_locked);
        assert!(s.rds.pi.is_none());
    }
}
