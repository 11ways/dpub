# Changelog

All notable changes to this project will be documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/) once it reaches 1.0.

## [Unreleased]

### Added

- ACE accessibility validation. New subcommand `dpub a11y <epub>` runs the [DAISY ACE checker](https://github.com/daisy/ace) (when `ace` is on PATH — install via `npm install -g @daisy/ace`) and prints a structured report. New `--a11y` flag on `dpub convert` runs the same check immediately after writing the EPUB. EPUBCheck validates spec compliance; ACE validates accessibility (WCAG via axe-core plus EPUB-specific a11y rules). Both matter under the European Accessibility Act. The exit code is non-zero on errors. ACE is opt-in; missing-binary path produces a clear install hint.
- `dpub convert --cover <path>` embeds a JPEG or PNG cover image into the produced EPUB. The image is referenced from the OPF manifest with the EPUB 3.3 `properties="cover-image"` form so any spec-compliant reader (Apple Books, Thorium, Calibre) shows it as the book's cover. Magic-byte sniffing only — no decode, no resize. Anything that isn't a JPEG or PNG is rejected loudly.

### Changed

- `dpub-whisper` exposes a new `Transcriber` struct that owns the loaded GGML model and offers a `transcribe(&self, audio_path)` method. `dpub-convert::inject_transcripts` now constructs one `Transcriber` per book and reuses it across every audio file, instead of re-loading the model per call. Closes #10. Saves ~3–5 minutes wallclock on a 30-section book and avoids per-file Metal/CUDA buffer churn. The free `dpub_whisper::transcribe(audio, opts)` function is preserved as a one-shot convenience for the smoke test.

### Added

- Whisper transcripts are now post-processed into prose-shaped paragraphs (~3–6 sentences each) before being injected into the EPUB content XHTMLs, instead of one `<p>` per Whisper segment. The merge is a single-pass greedy state machine with sentence-terminator detection, decimal-number / Dutch-abbreviation false-positive guards, and a max-character safety valve for hallucinated unpunctuated runs. Each cleaned paragraph carries a stable `id="tx-<section>-<para>"` so a future per-paragraph Media Overlay sync milestone can reference it without re-rendering the XHTML. Pass `--no-text-cleanup` to keep the raw per-segment output for debugging.
- `dpub-cli` and `dpub-convert` now expose `metal` and `cuda` Cargo features that forward to `dpub-whisper`. Build with `cargo build --release -p dpub-cli --features metal` on Apple Silicon to GPU-accelerate `--transcribe` runs (5–10× faster against medium / large-v3 models). Off by default so CI and no-GPU builds stay working.
- `dpub info` and `dpub convert` now accept either an `ncc.html` file or the directory containing it. Spec-mandated `ncc.html` is tried first; legacy uppercase variants (`NCC.HTML`) resolve via a case-insensitive directory scan. Missing-NCC directories produce a clear error instead of `EISDIR`.
- In-tree synthetic DAISY 2.02 fixture at `crates/dpub-convert/tests/fixtures/minimal_daisy/` (~10 KB total: NCC, master.smil, one section SMIL, one tiny MP3). Three integration tests exercise the full parse → convert → ZIP pipeline against it on every `cargo test` run, including CI. The optional EPUBCheck assertion fires when `epubcheck` is on PATH.
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
- New `dpub-validate` crate and `dpub validate <epub>` subcommand (M4): subprocess-wraps the official EPUBCheck JVM tool when present on `PATH`, parses its `--json -` output, and prints a structured summary plus per-issue list. The `dpub convert` command grows a `--validate` flag that runs validation right after writing. Exit code is non-zero when any error or fatal is reported.
- New `dpub-audio` crate and `dpub convert --audio opus --bitrate <kbps>` flags (M5): subprocess-wraps the system `ffmpeg` to re-encode every audio file to Ogg/Opus before EPUB assembly, with all Media Overlay clip times preserved unchanged (Opus stays in-seconds, just like MP3). On the reference 11 h 45 m audiobook, `--audio opus --bitrate 32` produces a **2.5× smaller** EPUB (159.9 MiB vs 403.6 MiB) that still validates EPUBCheck-clean. Defaults: 64 kbit/s, mono, 16 kHz, `voip` application — speech-tuned.

### Added (continued)

- New `dpub-whisper` crate and `dpub convert --transcribe <lang> --whisper-model <path>` flags (M6): runs local Whisper transcription via `whisper-rs` (FFI to whisper.cpp) on every audio file in the publication, then injects the time-ordered text as `<p>` paragraphs into each section's content XHTML. Audio decoding is pure-Rust (`symphonia` + `rubato` resampling to 16 kHz mono); no extra ffmpeg dependency for the transcription path. Models are GGML-format `ggml-*.bin` files downloaded separately from <https://huggingface.co/ggerganov/whisper.cpp>. Optional Cargo features `metal` / `cuda` enable GPU acceleration. Smoke-tested end-to-end against a synthetic MP3 with the `tiny` model.

### Changed (M5.5 consolidation)

- New `dpub-util` crate with `xml::escape_text` / `xml::escape_attr` returning `Cow<'_, str>` (zero allocation when no escapes are needed). Replaces three independent in-tree implementations.
- `epub3_writer::Error`: removed the `ZipIo(#[from] std::io::Error)` variant. The bare-`io::Error` `#[from]` was shadowing the path-aware `Io { path, source }` variant, so any `?` on a ZIP I/O failure used to silently lose the path that was being written. Every site now wraps with explicit context via a small `write_entry()` helper in `zip_assembly`.
- `dpub-convert::convert()` pre-buckets the NCC nav by SMIL filename so per-section content rendering is O(N) rather than O(sections × nav_items).
- Audio recompression in `dpub-convert` runs `ffmpeg` jobs in parallel via `rayon`. The 11 h 45 m reference book now re-encodes to Opus in **~98 s** wallclock instead of ~390 s — a 4× speedup driven by ~5-6 effective cores in use.
- `convert_to_file_with_options` is gone; there is now a single `convert_to_file(book, output, opts)` entry point that takes an explicit `ConvertOptions` value (use `ConvertOptions::default()` for "embed source audio unchanged").
- Tests: ~25 new unit tests covering malformed NCC handling, time-parser edge cases, audio media-type lookup, hierarchical nav levels, `Publication::validate`, and XML-escaper behaviour. Total test count is now 44 (was 20).
- README: new "Local development" section documenting the `DPUB_TEST_BOOK`, `DPUB_TEST_OPUS`, `epubcheck`, and `ffmpeg` opt-in switches.
- Rustdoc: filled in the previously-undocumented `Book` field meanings, `MediaOverlay::duration_seconds`, and `OverlaySeq::textref` (which has subtle path-relativity rules that EPUBCheck enforces).

[Unreleased]: https://github.com/11ways/dpub/compare/...HEAD
