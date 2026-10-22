# Changelog

All notable changes to FM Analyzer are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-10-22

### Added
- Stereo audio output to the system sound card (WASAPI / ALSA / CoreAudio via cpal).
- Stereo decoder locked to the 19 kHz pilot, automatic mono fallback, forced-mono switch.
- De-emphasis selection: 50 µs, 75 µs or off.
- Volume and mute controls, left/right audio peak meters, STEREO indicator.
- Status lamps for signal, stereo, RDS and over-modulation.
- Frequency entry by typing, mouse-wheel tuning and 10/50/100 kHz and 1 MHz step buttons.

### Changed
- New dark, instrument-style interface: segmented bar meters with colour zones and scales,
  LCD-style frequency and PS displays, card layout, dedicated RDS panel.

### Removed
- Built-in demo signal source (the synthetic test station is kept for the unit tests only).

### Fixed
- Pilot phase used for RDS/stereo demodulation was one sample late.

## [0.1.0] - 2026-10-22

### Added
- RTL-SDR sources: USB (librtlsdr loaded at runtime) and rtl_tcp network client.
- Built-in demo signal: stereo FM station on 98.00 MHz with pilot and RDS.
- FM channel filter, discriminator and MPX processing at 228 kS/s.
- Measurements: peak deviation with hold and >75 kHz percentage, pilot level and lock,
  RDS injection estimate, ITU-R BS.412 MPX power (60 s and 100 ms), channel level, carrier offset.
- RF spectrum, MPX spectrum and 60 s deviation history plots.
- RDS decoder: PI, PS, PTY, TP/TA/MS, RadioText, clock time, AF list, group statistics, BLER.
- GitHub Actions CI and tagged releases for Windows x64/x86, Linux x64 and macOS arm64.

[Unreleased]: ../../compare/v0.2.0...HEAD
[0.2.0]: ../../compare/v0.1.0...v0.2.0
[0.1.0]: ../../releases/tag/v0.1.0
