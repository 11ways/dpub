# dpub

A modern, native toolchain for [DAISY 2.02](https://daisy.org/activities/standards/daisy/) talking books and [EPUB 3](https://www.w3.org/TR/epub-33/) accessible publications. Single static binary. No JVM. No XSLT runtime.

> **Status:** M0.5–M6 shipped, plus the Tier 1 1.0-readiness items: ACE accessibility validation, automatic cover lookup, parallel batch conversion, and stable JSON output for CI/pipeline use. M7 (WASM) and M8 (1.0 release) are the remaining milestones.

## Why?

dpub produces production-ready EPUB 3 talking books from DAISY 2.02 sources, with **both spec compliance and accessibility validated by default**. It bundles [EPUBCheck](https://github.com/w3c/epubcheck) (the W3C tool for EPUB conformance) and [ACE](https://github.com/daisy/ace) (the DAISY Consortium's WCAG checker) so producers can ship books that pass both validators in one pipeline — relevant for EU accessibility production workflows including those affected by the [European Accessibility Act](https://ec.europa.eu/social/main.jsp?catId=1202).

The reference toolchain for this conversion is the [DAISY Pipeline 2](https://daisy.github.io/pipeline/) — excellent, mature, Java-based, ~150 MB of runtime, built on XSLT/XProc, and its `daisy202-to-epub3` script currently produces output with two known EPUBCheck errors plus 31 warnings on a real audiobook. dpub takes a different shape:

- **Production-ready by default** — output is EPUBCheck-clean (0 errors / 0 warnings) on the reference 11h45m audiobook where Pipeline 2's output emits 2 errors and 31 warnings on the same input. No downstream remediation work.
- **Validators bundled** — `dpub validate` and `dpub a11y` run EPUBCheck and ACE on any EPUB, including dpub's own output. Both surfaces emit stable JSON via `--json` for CI/pipeline use.
- **Audio-aware** — optional MP3 → Opus re-encoding (≈2.5× smaller files at speech-friendly settings), local Whisper transcription with prose-shaped paragraph cleanup, automatic cover lookup via Open Library, all within one binary.
- **Catalogue-ready** — `dpub batch` walks a directory and converts every DAISY book in parallel, with per-book error recovery and a JSON summary on stdout. Pipeline 2 has no batch mode.
- **Tiny native binary** — a single static binary, around 5 MB once 1.0 ships. No JVM startup. No XSLT runtime. Embeddable as a Rust library, with a planned WebAssembly build.

## Quickstart

Build from source (requires `cmake` for the bundled `dpub-whisper` crate):

```sh
git clone https://github.com/11ways/dpub
cd dpub
cargo build --release
```

### Inspect a DAISY book

```sh
./target/release/dpub info /path/to/daisy/
```

`dpub info` accepts either an `ncc.html` file directly or a directory containing one. Example output for a real Vlaams DAISY 2.02 audiobook:

```
Title:         Ontmoetingen in het donker
Creator:       Geertje De Ceuleneer
Date:          2008-05-13
Language:      nl
Multimedia:    audioNCC
Total time:    11:45:09

Navigation:
  Headings:    30 (h1: 19, h2: 11)
  Pages:       334

SMIL:
  Sections:    30
  Synch points: 364
  Audio clips: 10532
  Audio total: 11:45:09
  Audio files: 30
```

### Convert one book

```sh
./target/release/dpub convert /path/to/daisy/ -o book.epub
```

Add validation and audio recompression and a cover lookup, all in one pass:

```sh
./target/release/dpub convert /path/to/daisy/ \
    -o book.epub \
    --audio opus --bitrate 32 \
    --auto-cover \
    --validate --a11y
```

For audio-only DAISY books, transcribe to fill the text layer (requires a [GGML Whisper model](https://huggingface.co/ggerganov/whisper.cpp); enable Apple-Silicon Metal acceleration via `cargo build --release --features metal`):

```sh
./target/release/dpub convert /path/to/daisy/ \
    -o book.epub \
    --transcribe nl \
    --whisper-model ~/models/ggml-medium.bin
```

### Convert a whole catalogue in parallel

```sh
./target/release/dpub batch /path/to/daisy-books/ \
    -o /path/to/output/ \
    --jobs 4 \
    --audio opus --bitrate 32
```

Walks the input directory for every `ncc.html`, converts each book in parallel via rayon, emits a JSON summary on stdout. Per-book errors are recorded but never halt the queue. Exit code is non-zero when any book failed.

### Validate or accessibility-check on its own

```sh
./target/release/dpub validate book.epub
./target/release/dpub a11y book.epub
./target/release/dpub validate book.epub --json | jq .epubcheck.summary
```

`dpub validate` requires `epubcheck` on `PATH` (`brew install epubcheck`, or download from [w3c/epubcheck](https://github.com/w3c/epubcheck/releases)). `dpub a11y` requires `ace` (`npm install -g @daisy/ace`).

> **Tip — opening the converted EPUB.** Most generic readers (including Apple Books) display EPUB 3 but won't play [Media Overlays](https://www.w3.org/TR/epub-33/#sec-media-overlays), so an audio DAISY converted by `dpub` will look like a silent shell. Use [Thorium Reader](https://www.edrlab.org/software/thorium-reader/) (free, open-source, EDRLab) — the reference Media Overlays implementation — to get synced text-and-audio playback. On macOS: `brew install --cask thorium`.

## Roadmap

| Milestone | Scope |
| --- | --- |
| **M0.5** | `dpub info` — read NCC metadata and nav summary. ✅ |
| **M1** | Full DAISY parser: NCC + master.smil + per-section SMIL + audio metadata; structural round-trip. ✅ |
| **M2** | Minimal EPUB 3 writer (audio-only with Media Overlays). EPUBCheck-clean. ✅ |
| **M3** | End-to-end: `dpub convert <ncc.html> -o out.epub`. ✅ |
| **M4** | Built-in validation (EPUBCheck + ACE) — `dpub validate`, `dpub a11y`. ✅ |
| **M5** | Audio recompression (MP3 → Opus) — `dpub convert --audio opus --bitrate <kbps>`. ✅ |
| **M6** | Whisper transcription for audio-only books — `dpub convert --transcribe <lang> --whisper-model <path>`. ✅ (segments are merged into prose-shaped paragraphs by default; pass `--no-text-cleanup` for raw output) |
| **Tier 1 polish** | Whisper model caching, cover lookup (`--cover` and `--auto-cover`), parallel batch mode, JSON output for validators. ✅ |
| **M7** | WASM build for browser-based conversion (planned scope: `info` + `validate` only — Whisper / ffmpeg are too heavy for a browser tab). |
| **M8** | 1.0 release: signed binaries for macOS / Linux / Windows. |

## Project layout

```
dpub/
├── crates/
│   ├── dpub-core/      # DAISY 2.02 model + parser
│   ├── epub3-writer/   # EPUB 3 model + ZIP serialiser (Media Overlays, cover image)
│   ├── dpub-convert/   # DAISY 2.02 → EPUB 3 conversion driver
│   ├── dpub-validate/  # EPUBCheck + ACE wrappers, structured Report
│   ├── dpub-audio/     # ffmpeg-backed MP3 → Opus re-encoder
│   ├── dpub-whisper/   # local Whisper transcription (whisper.cpp via FFI)
│   ├── dpub-meta/      # external metadata lookup (Open Library covers)
│   ├── dpub-util/      # tiny shared utilities (XML escaping)
│   └── dpub-cli/       # `dpub` binary
└── ...
```

`dpub-wasm` lands with M7.

## Local development

A few opt-in environment variables turn on integration tests that need
external assets or are slow:

| Variable | Effect |
| --- | --- |
| `DPUB_TEST_BOOK=/path/to/ncc.html` | Enables the round-trip and conversion tests against a real DAISY book on disk. |
| `DPUB_TEST_OPUS=1` (with `DPUB_TEST_BOOK`) | Enables the full-book Opus re-encode test (slow — minutes). |
| `DPUB_TEST_WHISPER_MODEL=/path/ggml-*.bin` and `DPUB_TEST_AUDIO=/path/audio.mp3` | Enable the Whisper smoke test in `dpub-whisper`. Optional `DPUB_TEST_WHISPER_LANG=nl`. |
| `epubcheck` on `PATH` | The `dpub-validate` and `epub3-writer` integration tests will run EPUBCheck and assert zero errors. They skip silently if the binary is missing. |
| `ace` on `PATH` | (No CI-gated test yet.) Enables `dpub a11y` end-to-end. Install with `npm install -g @daisy/ace`. |
| `ffmpeg` on `PATH` | The `dpub-audio` and Opus re-encoding tests run; they skip silently otherwise. |
| `cmake` on `PATH` | Required to build `dpub-whisper` (and therefore `dpub-cli` once it depends on it). The `whisper-rs-sys` crate compiles whisper.cpp from source. |

Without any of these, `cargo test` still runs the full unit test suite
on every platform.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). All contributors are expected to follow the [`Code of Conduct`](CODE_OF_CONDUCT.md).

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](LICENSE-MIT))

at your option. This is the standard Rust dual-license, allowing maximum compatibility with downstream projects.

## Acknowledgements

The DAISY format and the [DAISY Consortium](https://daisy.org/) have been the global standard for accessible publications for decades. dpub stands on the shoulders of decades of work by the consortium and the [Pipeline 2](https://github.com/daisy/pipeline) team — we use their reference output as a correctness baseline.

Maintained by [Eleven Ways](https://www.elevenways.be/), a Belgian digital accessibility consultancy.
