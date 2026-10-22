//! I/Q sample sources: RTL-SDR over USB (librtlsdr, loaded at runtime)
//! and rtl_tcp network servers.

use std::io;

pub mod tcp;
#[cfg(feature = "usb")]
pub mod usb;

/// A source of interleaved unsigned 8-bit I/Q samples (RTL-SDR format).
pub trait IqSource: Send {
    fn name(&self) -> String;
    fn set_sample_rate(&mut self, hz: u32) -> io::Result<()>;
    fn set_center_freq(&mut self, hz: u32) -> io::Result<()>;
    /// `None` = automatic gain; `Some(g)` = manual gain in tenths of a dB.
    fn set_gain(&mut self, tenth_db: Option<i32>) -> io::Result<()>;
    fn set_ppm(&mut self, ppm: i32) -> io::Result<()>;
    /// Blocks until some samples are available. `Ok(0)` means end of stream.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
}

/// Gains offered by the R820T/R828D tuner, in dB.
pub const R820T_GAINS: [f32; 29] = [
    0.0, 0.9, 1.4, 2.7, 3.7, 7.7, 8.7, 12.5, 14.4, 15.7, 16.6, 19.7, 20.7, 22.9, 25.4, 28.0,
    29.7, 32.8, 33.8, 36.4, 37.2, 38.6, 40.2, 42.1, 43.4, 43.9, 44.5, 48.0, 49.6,
];
