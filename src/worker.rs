//! Background thread: reads I/Q from a source, runs the analyzer,
//! publishes snapshots and applies tuning commands.

use crate::audio::{resample_step, AudioOut, Resampler};
use crate::fm::{Analyzer, Snapshot, IQ_RATE};
use crate::source::IqSource;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    pub freq_hz: u32,
    /// `None` = auto, otherwise tenths of a dB.
    pub gain: Option<i32>,
    pub ppm: i32,
    /// De-emphasis in microseconds (50 / 75 / 0 = off).
    pub deemphasis_us: f32,
    pub force_mono: bool,
}

pub enum Cmd {
    SetFreq(u32),
    SetGain(Option<i32>),
    SetPpm(i32),
    SetDeemphasis(f32),
    SetMono(bool),
    ResetRds,
    Stop,
}

#[derive(Default)]
pub struct Shared {
    pub snapshot: Mutex<Option<Snapshot>>,
    pub status: Mutex<String>,
    pub running: Mutex<bool>,
}

pub struct Worker {
    tx: Sender<Cmd>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(
        mut src: Box<dyn IqSource>,
        tuning: Tuning,
        shared: Arc<Shared>,
        audio: Arc<AudioOut>,
        notify: Box<dyn Fn() + Send>,
    ) -> Worker {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("fm-analyzer-dsp".into())
            .spawn(move || run(&mut *src, tuning, &shared, &audio, rx, notify))
            .expect("failed to spawn DSP thread");
        Worker { tx, handle: Some(handle) }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn stop(mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn set_status(shared: &Shared, s: impl Into<String>) {
    *shared.status.lock().unwrap() = s.into();
}

fn run(
    src: &mut dyn IqSource,
    t: Tuning,
    shared: &Shared,
    audio: &AudioOut,
    rx: Receiver<Cmd>,
    notify: Box<dyn Fn() + Send>,
) {
    let setup = (|| {
        src.set_sample_rate(IQ_RATE)?;
        src.set_ppm(t.ppm)?;
        src.set_center_freq(t.freq_hz)?;
        src.set_gain(t.gain)
    })();
    if let Err(e) = setup {
        set_status(shared, format!("Setup failed: {e}"));
        notify();
        return;
    }
    *shared.running.lock().unwrap() = true;
    set_status(shared, format!("Running: {}", src.name()));
    notify();

    let mut analyzer = Analyzer::new();
    analyzer.set_deemphasis(t.deemphasis_us);
    analyzer.set_force_mono(t.force_mono);
    let mut resampler = Resampler::new();
    let mut frames: Vec<[f32; 2]> = Vec::with_capacity(16_384);
    let mut pcm: Vec<f32> = Vec::with_capacity(32_768);
    // ~28 ms of samples per read
    let mut buf = vec![0u8; 131_072];
    loop {
        loop {
            match rx.try_recv() {
                Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => {
                    *shared.running.lock().unwrap() = false;
                    set_status(shared, "Stopped");
                    notify();
                    return;
                }
                Ok(cmd) => {
                    let r = match cmd {
                        Cmd::SetFreq(f) => {
                            analyzer.reset_station();
                            audio.clear();
                            src.set_center_freq(f)
                        }
                        Cmd::SetGain(g) => src.set_gain(g),
                        Cmd::SetPpm(p) => src.set_ppm(p),
                        Cmd::SetDeemphasis(us) => {
                            analyzer.set_deemphasis(us);
                            Ok(())
                        }
                        Cmd::SetMono(m) => {
                            analyzer.set_force_mono(m);
                            Ok(())
                        }
                        Cmd::ResetRds => {
                            analyzer.reset_station();
                            Ok(())
                        }
                        Cmd::Stop => unreachable!(),
                    };
                    if let Err(e) = r {
                        set_status(shared, format!("Command failed: {e}"));
                    }
                }
                Err(TryRecvError::Empty) => break,
            }
        }

        match src.read(&mut buf) {
            Ok(0) => {
                set_status(shared, "Stream ended");
                break;
            }
            Ok(n) => {
                let active = audio.active();
                analyzer.set_audio_enabled(active);
                let snaps = analyzer.process(&buf[..n]);
                if active {
                    frames.clear();
                    analyzer.drain_audio(&mut frames);
                    pcm.clear();
                    let step = resample_step(audio.rate(), audio.fill_secs());
                    resampler.process(&frames, step, &mut pcm);
                    audio.push(&pcm);
                }
                if let Some(snap) = snaps.into_iter().last() {
                    *shared.snapshot.lock().unwrap() = Some(snap);
                    notify();
                }
            }
            Err(e) => {
                set_status(shared, format!("Read error: {e}"));
                break;
            }
        }
    }
    *shared.running.lock().unwrap() = false;
    notify();
}
