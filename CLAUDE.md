# CLAUDE.md — agent / contributor briefing

**dpub** is a native Rust replacement for the Java/XSLT-based [DAISY Pipeline 2](https://daisy.github.io/pipeline/) `daisy202-to-epub3` converter. Goal: a single static binary that produces EPUB 3 (with Media Overlays) from DAISY 2.02 — faster, cleaner, no JVM. Maintained by [Eleven Ways](https://www.elevenways.be/) (Belgian a11y consultancy). Dual-licensed MIT / Apache-2.0.

## Layout

```
crates/
├── dpub-core/      # DAISY 2.02 parser + in-memory model (NCC, master.smil, per-section SMIL)
├── epub3-writer/   # EPUB 3 ZIP serialiser with Media Overlays
├── dpub-convert/   # DAISY 2.02 → EPUB 3 (drives the pipeline; --audio, --transcribe live here)
├── dpub-validate/  # EPUBCheck wrapper (subprocess + JSON parse)
├── dpub-audio/     # ffmpeg-backed MP3 → Opus re-encoder
├── dpub-whisper/   # local Whisper transcription via whisper.cpp (FFI)
├── dpub-util/      # shared XML escapers (used by 3 crates — don't reinvent)
└── dpub-cli/       # `dpub` binary
```

## Quickstart

```sh
cargo build --release
./target/release/dpub info  /path/to/ncc.html
./target/release/dpub convert /path/to/ncc.html -o out.epub \
    [--validate] \
    [--audio opus --bitrate 32] \
    [--transcribe nl --whisper-model ~/models/ggml-medium.bin]
./target/release/dpub validate /path/to/some.epub
```

## Conventions

- **Edition 2024, MSRV Rust 1.88** (let-chains). Workspace pins this; bump only with a CHANGELOG line.
- **Lints**: `forbid(unsafe_code)` workspace-wide; `clippy::pedantic` is on. Allows we explicitly opted into are listed in the workspace `Cargo.toml` `[workspace.lints.clippy]` block.
- **Tests**: `cargo test` must stay green on Linux/macOS/Windows. Don't use `--all-features` in CI — `dpub-whisper`'s `metal`/`cuda` features try to build whisper.cpp against GPU SDKs not present on runners.
- **PR flow**: feature branch → PR → CI green → squash-merge. Never push directly to `main`.
- **Work flow**: when implementing a milestone, tick boxes in `README.md`'s roadmap and add a `### Added` / `### Changed` line in `CHANGELOG.md` under `[Unreleased]`.

## External tools (each optional, each enables a slice of tests)

| Tool | Used by | How to install (macOS) | Notes |
|---|---|---|---|
| `cmake` | building `dpub-whisper` (whisper-rs-sys) | `brew install cmake` | **required to build the workspace** |
| `epubcheck` | `dpub validate`, M2/M4 integration tests | `brew install epubcheck` | needs Java 11 — keep `openjdk@11` on PATH |
| `ffmpeg` | `dpub convert --audio opus`, M5 tests | `brew install ffmpeg` | |
| Whisper GGML model | `dpub convert --transcribe ...` | download from <https://huggingface.co/ggerganov/whisper.cpp> | not bundled; `tiny` 75 MB → `large-v3` 1 GB |

## Opt-in test env vars

```sh
DPUB_TEST_BOOK=/path/to/ncc.html               # real-book parse + e2e tests (dpub-core, dpub-convert)
DPUB_TEST_OPUS=1                                # full-book Opus encode test (slow — ~minute on M-series with parallel ffmpeg)
DPUB_TEST_WHISPER_MODEL=/path/to/ggml-tiny.bin  # whisper smoke test (dpub-whisper)
DPUB_TEST_AUDIO=/path/to/some.mp3               # ↑ paired with this
```

## Test fixtures

- **In-tree synthetic fixtures** live under `crates/*/tests/fixtures/` — small, redistributable, exercised on every CI run. Add new ones here whenever a parser or writer edge case can be reproduced without a copyrighted real book.
- **Real DAISY books** are not redistributable (audio is third-party copyright), so the integration tests that exercise them are gated on `DPUB_TEST_BOOK=/path/to/ncc.html`. The reference book this project was bootstrapped against is "Ontmoetingen in het donker" (Vlaams, audio-only, ~11 h, 30 sections); machine-local paths to it live in agent memory, not here.
- **Pipeline 2 ground truth**: when comparing semantic equivalence to the official Java toolchain, set up Pipeline 2 separately and convert the same input. Path/install details belong in agent memory (per-machine) — `dpub` itself never depends on Pipeline 2 being installed.

## Roadmap status

See `README.md` for the canonical table. As of this writing: **M0.5–M6 done** (everything in `main`); **M7 (WASM)** and **M8 (1.0 release)** remain.

## Known correctness baseline

Conversion of the reference book is **EPUBCheck-clean (0/0/0)**, while DAISY Pipeline 2's output of the same input has **2 errors + 31 warnings** (`OPF-027` undefined `a11y:pageBreakSource`; `RSC-005` empty `master.smil` body; 30× missing `<title>` in section XHTMLs). Don't regress these guarantees.
