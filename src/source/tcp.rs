//! Client for the `rtl_tcp` protocol (also spoken by many SDR servers).
//!
//! On connect the server sends a 12-byte header: "RTL0", tuner type (u32 BE),
//! gain count (u32 BE). Commands are 5 bytes: command id + u32 BE parameter.
//! After that the server streams raw u8 I/Q.

use super::IqSource;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub struct RtlTcpSource {
    stream: TcpStream,
    addr: String,
    pub tuner_type: u32,
    pub gain_count: u32,
}

impl RtlTcpSource {
    pub fn connect(addr: &str) -> io::Result<Self> {
        let sock = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "address not resolved"))?;
        let mut stream = TcpStream::connect_timeout(&sock, Duration::from_secs(5))?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut hdr = [0u8; 12];
        stream.read_exact(&mut hdr)?;
        if &hdr[0..4] != b"RTL0" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an rtl_tcp server (missing RTL0 header)",
            ));
        }
        Ok(Self {
            stream,
            addr: addr.to_string(),
            tuner_type: u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]),
            gain_count: u32::from_be_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]),
        })
    }

    pub fn tuner_name(&self) -> &'static str {
        match self.tuner_type {
            1 => "E4000",
            2 => "FC0012",
            3 => "FC0013",
            4 => "FC2580",
            5 => "R820T",
            6 => "R828D",
            _ => "unknown",
        }
    }

    fn command(&mut self, cmd: u8, param: u32) -> io::Result<()> {
        let p = param.to_be_bytes();
        self.stream.write_all(&[cmd, p[0], p[1], p[2], p[3]])
    }
}

impl IqSource for RtlTcpSource {
    fn name(&self) -> String {
        format!("rtl_tcp {} ({})", self.addr, self.tuner_name())
    }
    fn set_sample_rate(&mut self, hz: u32) -> io::Result<()> {
        self.command(0x02, hz)
    }
    fn set_center_freq(&mut self, hz: u32) -> io::Result<()> {
        self.command(0x01, hz)
    }
    fn set_gain(&mut self, tenth_db: Option<i32>) -> io::Result<()> {
        match tenth_db {
            None => {
                self.command(0x03, 0)?;
                self.command(0x08, 1) // RTL2832 digital AGC on
            }
            Some(g) => {
                self.command(0x08, 0)?;
                self.command(0x03, 1)?;
                self.command(0x04, g as u32)
            }
        }
    }
    fn set_ppm(&mut self, ppm: i32) -> io::Result<()> {
        self.command(0x05, ppm as u32)
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}
