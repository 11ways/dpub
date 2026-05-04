# Changelog

All notable changes to this project will be documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/) once it reaches 1.0.

## [Unreleased]

### Added

- Initial Cargo workspace with `dpub-core` and `dpub-cli` crates.
- `dpub info <ncc.html>` reads metadata and navigation summary from a DAISY 2.02 publication (M0.5).
- `Ncc` parser handles XHTML 1.0 NCCs, including `windows-1252` and `iso-8859-1` legacy encodings.
- Dual MIT / Apache-2.0 licensing.
- Full DAISY 2.02 SMIL 1.0 parser: `MasterSmil`, `SectionSmil`, `SmilSeq`, `SmilPar`, `AudioClip`, `TextRef` (M1).
- SMIL clock-value parser (`time::parse_clock_value`) handling `npt=` prefix, full/partial colon forms, and timecount with `ms`/`s`/`min`/`h` units.
- `Book::from_ncc` now loads `master.smil` and every per-section SMIL.
- `Book::total_audio_seconds`, `total_par_count`, `total_audio_clip_count`, `audio_files`.
- Canonical-form SMIL writer (`write_master_smil`, `write_section_smil`); structural round-trip is tested against synthetic and real-world fixtures.
- Opt-in integration test (`DPUB_TEST_BOOK=...`) that asserts: section count matches `master.smil`, par count matches `ncc:tocItems`, audio total matches `ncc:totalTime` ±2s, and round-trip preserves the AST for every SMIL file in the book.
- `dpub info` now reports SMIL stats: section count, sync-point count, audio-clip count, total measured audio duration, distinct audio file count.

[Unreleased]: https://github.com/11ways/dpub/compare/...HEAD
