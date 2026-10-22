//! Persistent user settings in `settings.json`.
//!
//! The file lives next to the executable (portable use). If that folder is not
//! writable (e.g. Program Files), it falls back to the per-user config folder:
//! `%APPDATA%\FM Analyzer` on Windows, `~/.config/fm-analyzer` elsewhere.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

const FILE_NAME: &str = "settings.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "usb" or "tcp"
    pub source: String,
    pub host: String,
    pub port: u16,
    pub usb_index: u32,
    pub freq_mhz: f64,
    pub gain_auto: bool,
    pub gain_index: usize,
    pub ppm: i32,
    pub deemphasis_us: f32,
    pub force_mono: bool,
    pub volume: f32,
    pub muted: bool,
    pub calibration_db: f32,
    pub scan_start_khz: u32,
    pub scan_stop_khz: u32,
    pub scan_threshold_db: f32,
    pub scan_read_rds: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            source: if cfg!(feature = "usb") { "usb".into() } else { "tcp".into() },
            host: "127.0.0.1".into(),
            port: 1234,
            usb_index: 0,
            freq_mhz: 98.0,
            gain_auto: true,
            gain_index: 16,
            ppm: 0,
            deemphasis_us: 50.0,
            force_mono: false,
            volume: 0.7,
            muted: false,
            calibration_db: 107.0,
            scan_start_khz: 87_500,
            scan_stop_khz: 108_000,
            scan_threshold_db: 10.0,
            scan_read_rds: true,
        }
    }
}

fn exe_dir_path() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|d| d.join(FILE_NAME))
}

fn user_dir_path() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("FM Analyzer"));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("fm-analyzer"));
    base.map(|d| d.join(FILE_NAME))
}

impl Settings {
    /// Loads settings, preferring the file next to the executable.
    /// Returns the defaults if no file exists or it cannot be parsed.
    pub fn load() -> (Settings, Option<PathBuf>) {
        for path in [exe_dir_path(), user_dir_path()].into_iter().flatten() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(s) = serde_json::from_str::<Settings>(&text) {
                    return (s, Some(path));
                }
            }
        }
        (Settings::default(), None)
    }

    /// Saves to `preferred` (the file it was loaded from), else next to the executable,
    /// else to the user config folder. Returns the path written.
    pub fn save(&self, preferred: Option<&PathBuf>) -> io::Result<PathBuf> {
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let mut last_err = io::Error::other("no settings location available");
        let candidates = [preferred.cloned(), exe_dir_path(), user_dir_path()];
        for path in candidates.into_iter().flatten() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("json.tmp");
            let r = std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, &path));
            match r {
                Ok(()) => return Ok(path),
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }
}
