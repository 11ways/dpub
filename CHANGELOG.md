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
- New `epub3-writer` crate: typed `Publication` model and ZIP serialiser for accessible EPUB 3 publications with Media Overlays (M2). Output validates EPUBCheck-clean on synthetic fixtures; integration test invokes `epubcheck` automatically when present.
- Dependencies pinned at workspace level: `zip` 2.x, `chrono` 0.4 (clock-only feature), `uuid` 1.x.
- New `dpub-convert` crate and `dpub convert <ncc.html> -o out.epub` subcommand (M3): end-to-end DAISY 2.02 → EPUB 3 conversion. Audio is embedded byte-for-byte (no recompression); SMIL 1.0 Media Overlays are translated to SMIL 3.0; NCC navigation becomes a hierarchical `nav.xhtml` with a separate page-list. Verified against a real 11h45m audiobook: full conversion in <0.5s on M-series hardware, EPUBCheck-clean (0 errors / 0 warnings) where Pipeline 2's output emits 2 errors / 31 warnings on the same input.
- MSRV bumped to Rust 1.88 (let-chains).

[Unreleased]: https://github.com/11ways/dpub/compare/...HEAD
