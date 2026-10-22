# Changelog

All notable changes to FM Analyzer are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: ../../compare/v0.1.0...HEAD
[0.1.0]: ../../releases/tag/v0.1.0
