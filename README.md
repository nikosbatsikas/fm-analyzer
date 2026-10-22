# FM Analyzer

A portable FM broadcast analyzer written in Rust, in the spirit of tools like FM Scope.
It works with a cheap **RTL-SDR** dongle, either plugged in over **USB** or over the network via **rtl_tcp**,
and runs on **Windows x86 (32-bit) and x64**, Linux and macOS.

## Features

| Measurement | Details |
|---|---|
| Peak deviation | Per 100 ms, peak hold, % of samples above 75 kHz, 60 s history graph |
| Pilot (19 kHz) | Injection level in kHz, PLL lock, pilot frequency |
| RDS (57 kHz) | Injection level (approx.), sync, block error rate |
| MPX power | ITU-R BS.412, 60 s integration and 100 ms instantaneous, in dBr |
| Signal | Channel level and band level in dBFS, carrier offset |
| Spectra | RF spectrum (±1.14 MHz), MPX baseband spectrum 0–114 kHz with 19/38/57 kHz markers |
| RDS decoding | PI, PS, PTY, TP/TA/MS, RadioText (2A/2B), clock time (4A), AF list, group statistics |
| Audio | Stereo listening through the sound card, pilot-locked stereo decoder, 50/75 µs de-emphasis, L/R peak meters |

Sources:

* **RTL-SDR USB** – uses `librtlsdr`, loaded at runtime (nothing is linked at build time, which keeps
  32/64-bit builds simple).
* **rtl_tcp** – connect to any `rtl_tcp` server on your LAN or the internet.

## Quick start

Download the zip for your system from the [Releases](../../releases) page, unzip it and run
`fm-analyzer.exe` (Windows) or `fm-analyzer` (Linux/macOS). Pick **USB** or **RTL-TCP**, press
**CONNECT**, then tune:

* click the frequency display and type (`98.5`, `985` or `98500` all work), or
* scroll the mouse wheel over it, use the step buttons, or the ←/→ keys (100 kHz).

Audio plays on the system's default output device. Use **ON/MUTED** and the volume slider in the
*Audio* section, **Stereo/Mono**, and pick **50 µs** de-emphasis for Europe or **75 µs** for the
Americas.

### Using a USB dongle

**Windows**

1. Install the WinUSB driver for the dongle with [Zadig](https://zadig.akeo.ie/)
   (Options → List all devices → "Bulk-In, Interface (Interface 0)" → WinUSB).
2. Put `rtlsdr.dll` and `libusb-1.0.dll` next to `fm-analyzer.exe`.
   **They must match the app's architecture**: 32-bit DLLs for the x86 build, 64-bit DLLs for the
   x64 build. Builds are available from the RTL-SDR Blog driver releases (x32/x64 folders) and from
   the osmocom rtl-sdr project.

**Linux**: `sudo apt install librtlsdr0` (or `rtl-sdr`), and blacklist the DVB driver
(`dvb_usb_rtl28xxu`) if the device is busy.

**macOS**: `brew install librtlsdr`.

### Using rtl_tcp

On the machine with the dongle:

```sh
rtl_tcp -a 0.0.0.0 -p 1234
```

In FM Analyzer choose **RTL-TCP**, enter the host and port and press **CONNECT**. The app sets the
sample rate (2.28 MS/s), frequency, gain and PPM itself. Note that 2.28 MS/s of 8-bit I/Q is about
4.6 MB/s (≈37 Mbit/s), so use a wired LAN.

## Building from source

Requires Rust 1.80 or newer (`rustup` from <https://rustup.rs>).

```sh
cargo run --release                     # build and run for this machine
cargo test --release --lib              # DSP, RDS and stereo tests on a synthetic station
```

Windows 32-bit and 64-bit:

```sh
rustup target add i686-pc-windows-msvc x86_64-pc-windows-msvc
cargo build --release --target i686-pc-windows-msvc     # x86 (32-bit)
cargo build --release --target x86_64-pc-windows-msvc   # x64
```

Linux build dependencies (Debian/Ubuntu):

```sh
sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libgl1-mesa-dev libwayland-dev libasound2-dev
```

Optional features (both on by default): `usb` (RTL-SDR over USB) and `audio` (sound output).
For example `cargo build --release --no-default-features --features audio` builds an rtl_tcp-only
version.

## How it works

```
RTL-SDR 2.28 MS/s u8 I/Q
  └─ channel filter ±110 kHz (255-tap FIR) + decimate ÷10 → 228 kS/s
       └─ FM discriminator  arg(z[n]·conj(z[n-1])) → MPX in Hz of deviation
            ├─ peak / >75 kHz / BS.412 MPX power
            ├─ MPX FFT (4096)
            ├─ 19 kHz PLL → pilot level & lock
            ├─ L+R and (L−R)·(−2·sin 2θ) → 15 kHz low-pass ÷4 → 57 kS/s → de-emphasis
            │    └─ cubic resampler (drift-controlled) → sound card
            └─ ×e^(-j·3θ) (57 kHz locked to pilot) → low-pass + ÷12 → 19 kS/s
                 └─ BPSK phase align → biphase matched filter + timing → differential decode
                      └─ block sync (CRC syndromes A/B/C/C'/D) → group decoder
```

Source layout:

| File | Purpose |
|---|---|
| `src/dsp.rs` | Complex math, FIR design, decimating FIR, FFT, spectrum averaging, pilot PLL |
| `src/fm.rs` | The analyzer: measurements and spectra, produces a `Snapshot` every 100 ms |
| `src/rds.rs` | RDS demodulator, block synchroniser and group decoder |
| `src/audio.rs` | Stereo decoder, de-emphasis, resampler and audio FIFO |
| `src/source/` | `usb.rs` (librtlsdr), `tcp.rs` (rtl_tcp client) |
| `src/worker.rs` | Background DSP thread and command channel |
| `src/testsignal.rs` | Synthetic stereo + RDS station used by the unit tests |
| `src/main.rs`, `src/ui/` | GUI (egui / eframe): layout, dark theme, meters and widgets |
| `src/sound.rs` | Sound-card output (cpal) |

The core library (`dsp`, `fm`, `rds`, sources) has no dependencies besides the optional
`libloading`, so it can be reused in other tools (e.g. a headless monitor or logger).

### About accuracy

Deviation, pilot and RDS levels are derived from frequency, so they are as accurate as the dongle's
crystal (a few ppm – negligible). Levels in dBFS are **not** calibrated to dBµV/dBf. Peak deviation
is sensitive to noise and multipath: on weak signals the peak reading rises. The RDS level is an
estimate from the RMS of the 57 kHz subcarrier. For compliance-grade measurements use a calibrated
receiver; this tool is meant for monitoring and comparison.

## Versioning and releases

The project follows [Semantic Versioning](https://semver.org/) and keeps a
[CHANGELOG](CHANGELOG.md) in the *Keep a Changelog* format.

To publish a release:

1. Move the items under `## [Unreleased]` in `CHANGELOG.md` into a new `## [X.Y.Z] - YYYY-MM-DD` section.
2. Set `version = "X.Y.Z"` in `Cargo.toml`.
3. Commit, tag and push:

   ```sh
   git commit -am "Release vX.Y.Z"
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin main --follow-tags
   ```

GitHub Actions then tests and builds Windows x64, Windows x86, Linux x64 and macOS arm64, and
publishes a GitHub Release with the zips and the changelog notes. The workflow refuses to release if
the tag does not match the version in `Cargo.toml`. `0.x` versions are marked as pre-releases.

## Roadmap

* Output device selection, audio recording to WAV
* Band scan with a station list (frequency, level, PI, PS)
* CSV logging of measurements, deviation histogram
* More RDS: PTYN, RT+, EON, long PS, Greek/extended character tables
* Multipath / ASU indicator, recording and playback of I/Q files

## License

MIT – see [LICENSE](LICENSE).
