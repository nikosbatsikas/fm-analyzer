//! RTL-SDR over USB via librtlsdr, loaded at runtime with `libloading`.
//!
//! Nothing is linked at build time, so the same code builds for 32-bit and
//! 64-bit Windows/Linux/macOS. At runtime the app looks for:
//!   Windows: rtlsdr.dll / librtlsdr.dll (next to the .exe or on PATH; must match the
//!            app's bitness, plus libusb-1.0.dll; device needs the WinUSB driver via Zadig)
//!   Linux:   librtlsdr.so.0 / librtlsdr.so (package `librtlsdr0` / `rtl-sdr`)
//!   macOS:   librtlsdr.dylib / librtlsdr.0.dylib (Homebrew `librtlsdr`)

use super::IqSource;
use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::io;

#[repr(C)]
struct RtlDev {
    _private: [u8; 0],
}

type FnCount = unsafe extern "C" fn() -> u32;
type FnName = unsafe extern "C" fn(u32) -> *const c_char;
type FnOpen = unsafe extern "C" fn(*mut *mut RtlDev, u32) -> c_int;
type FnClose = unsafe extern "C" fn(*mut RtlDev) -> c_int;
type FnSetU32 = unsafe extern "C" fn(*mut RtlDev, u32) -> c_int;
type FnSetInt = unsafe extern "C" fn(*mut RtlDev, c_int) -> c_int;
type FnReset = unsafe extern "C" fn(*mut RtlDev) -> c_int;
type FnReadSync = unsafe extern "C" fn(*mut RtlDev, *mut c_void, c_int, *mut c_int) -> c_int;

struct Api {
    _lib: Library,
    get_device_count: FnCount,
    get_device_name: FnName,
    open: FnOpen,
    close: FnClose,
    set_center_freq: FnSetU32,
    set_sample_rate: FnSetU32,
    set_tuner_gain_mode: FnSetInt,
    set_tuner_gain: FnSetInt,
    set_freq_correction: FnSetInt,
    set_agc_mode: FnSetInt,
    reset_buffer: FnReset,
    read_sync: FnReadSync,
}

#[cfg(target_os = "windows")]
const LIB_NAMES: &[&str] = &["rtlsdr.dll", "librtlsdr.dll"];
#[cfg(target_os = "macos")]
const LIB_NAMES: &[&str] = &["librtlsdr.dylib", "librtlsdr.0.dylib", "/opt/homebrew/lib/librtlsdr.dylib", "/usr/local/lib/librtlsdr.dylib"];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const LIB_NAMES: &[&str] = &["librtlsdr.so.0", "librtlsdr.so", "librtlsdr.so.2"];

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

impl Api {
    fn load() -> io::Result<Api> {
        let mut last = String::new();
        for name in LIB_NAMES {
            // SAFETY: loading a well-known C library; its initialisers are trusted.
            match unsafe { Library::new(name) } {
                Ok(lib) => return Self::bind(lib),
                Err(e) => last = format!("{name}: {e}"),
            }
        }
        Err(err(format!(
            "librtlsdr not found ({last}). Install the RTL-SDR driver library \
             matching this app's architecture ({}-bit).",
            usize::BITS
        )))
    }

    fn bind(lib: Library) -> io::Result<Api> {
        macro_rules! sym {
            ($name:literal, $t:ty) => {
                // SAFETY: the symbol types match the librtlsdr C API (rtl-sdr.h).
                *unsafe { lib.get::<$t>(concat!($name, "\0").as_bytes()) }
                    .map_err(|e| err(format!("missing symbol {}: {e}", $name)))?
            };
        }
        Ok(Api {
            get_device_count: sym!("rtlsdr_get_device_count", FnCount),
            get_device_name: sym!("rtlsdr_get_device_name", FnName),
            open: sym!("rtlsdr_open", FnOpen),
            close: sym!("rtlsdr_close", FnClose),
            set_center_freq: sym!("rtlsdr_set_center_freq", FnSetU32),
            set_sample_rate: sym!("rtlsdr_set_sample_rate", FnSetU32),
            set_tuner_gain_mode: sym!("rtlsdr_set_tuner_gain_mode", FnSetInt),
            set_tuner_gain: sym!("rtlsdr_set_tuner_gain", FnSetInt),
            set_freq_correction: sym!("rtlsdr_set_freq_correction", FnSetInt),
            set_agc_mode: sym!("rtlsdr_set_agc_mode", FnSetInt),
            reset_buffer: sym!("rtlsdr_reset_buffer", FnReset),
            read_sync: sym!("rtlsdr_read_sync", FnReadSync),
            _lib: lib,
        })
    }
}

/// Lists connected RTL-SDR devices.
pub fn list_devices() -> io::Result<Vec<String>> {
    let api = Api::load()?;
    // SAFETY: plain C calls with valid arguments.
    unsafe {
        let n = (api.get_device_count)();
        Ok((0..n)
            .map(|i| {
                let p = (api.get_device_name)(i);
                let name = if p.is_null() {
                    "RTL-SDR".to_string()
                } else {
                    CStr::from_ptr(p).to_string_lossy().into_owned()
                };
                format!("#{i}: {name}")
            })
            .collect())
    }
}

pub struct UsbSource {
    api: Api,
    dev: *mut RtlDev,
    index: u32,
    started: bool,
}

// SAFETY: the device handle is only used from the thread that owns the source.
unsafe impl Send for UsbSource {}

impl UsbSource {
    pub fn open(index: u32) -> io::Result<Self> {
        let api = Api::load()?;
        let mut dev: *mut RtlDev = std::ptr::null_mut();
        // SAFETY: `dev` is a valid out-pointer.
        let r = unsafe { (api.open)(&mut dev, index) };
        if r < 0 || dev.is_null() {
            return Err(err(format!(
                "rtlsdr_open(#{index}) failed ({r}). Is the dongle plugged in, not used by \
                 another program, and (on Windows) using the WinUSB driver from Zadig?"
            )));
        }
        Ok(Self { api, dev, index, started: false })
    }

    fn check(r: c_int, what: &str) -> io::Result<()> {
        if r < 0 {
            Err(err(format!("{what} failed ({r})")))
        } else {
            Ok(())
        }
    }
}

impl IqSource for UsbSource {
    fn name(&self) -> String {
        format!("RTL-SDR USB #{}", self.index)
    }
    fn set_sample_rate(&mut self, hz: u32) -> io::Result<()> {
        // SAFETY: valid open device.
        Self::check(unsafe { (self.api.set_sample_rate)(self.dev, hz) }, "set_sample_rate")
    }
    fn set_center_freq(&mut self, hz: u32) -> io::Result<()> {
        Self::check(unsafe { (self.api.set_center_freq)(self.dev, hz) }, "set_center_freq")
    }
    fn set_gain(&mut self, tenth_db: Option<i32>) -> io::Result<()> {
        unsafe {
            match tenth_db {
                None => {
                    Self::check((self.api.set_tuner_gain_mode)(self.dev, 0), "gain mode")?;
                    Self::check((self.api.set_agc_mode)(self.dev, 1), "agc mode")
                }
                Some(g) => {
                    Self::check((self.api.set_agc_mode)(self.dev, 0), "agc mode")?;
                    Self::check((self.api.set_tuner_gain_mode)(self.dev, 1), "gain mode")?;
                    Self::check((self.api.set_tuner_gain)(self.dev, g), "set_tuner_gain")
                }
            }
        }
    }
    fn set_ppm(&mut self, ppm: i32) -> io::Result<()> {
        // librtlsdr returns -2 when the value is unchanged; that is not an error.
        let r = unsafe { (self.api.set_freq_correction)(self.dev, ppm) };
        if r == -2 {
            Ok(())
        } else {
            Self::check(r, "set_freq_correction")
        }
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.started {
            Self::check(unsafe { (self.api.reset_buffer)(self.dev) }, "reset_buffer")?;
            self.started = true;
        }
        // librtlsdr wants a multiple of 512 bytes
        let len = (buf.len() / 512 * 512).min(i32::MAX as usize) as c_int;
        let mut n: c_int = 0;
        let r = unsafe {
            (self.api.read_sync)(self.dev, buf.as_mut_ptr() as *mut c_void, len, &mut n)
        };
        Self::check(r, "read_sync")?;
        Ok(n.max(0) as usize)
    }
}

impl Drop for UsbSource {
    fn drop(&mut self) {
        if !self.dev.is_null() {
            // SAFETY: device was opened by us and is closed exactly once.
            unsafe { (self.api.close)(self.dev) };
            self.dev = std::ptr::null_mut();
        }
    }
}
