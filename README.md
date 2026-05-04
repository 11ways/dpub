# dpub

A modern, native toolkit for [DAISY 2.02](https://daisy.org/activities/standards/daisy/) talking books and [EPUB 3](https://www.w3.org/TR/epub-33/) accessible publications. Single static binary. No JVM. No XSLT runtime.

> **Status:** very early. The first milestone (`dpub info`) ships in the initial commit. The roadmap below is what we're building toward.

## Why?

The reference toolchain for DAISY ↔ EPUB 3 conversion is the [DAISY Pipeline 2](https://daisy.github.io/pipeline/) — excellent, mature, but Java-based, ~150 MB of runtime, and built on XSLT/XProc. We wanted something:

- **Tiny** — a single static binary, around 5 MB.
- **Fast** — no JVM startup, native parsing.
- **Embeddable** — run in the browser via WebAssembly, or as a library from Rust/Python/Node.
- **Stricter by default** — output is EPUBCheck-clean; Pipeline 2's `daisy202-to-epub3` currently produces output with two known EPUBCheck errors.
- **Extended** — built-in validation, audio recompression (MP3 → Opus), and Whisper transcription so audio-only DAISY books gain a searchable text layer in the resulting EPUB.

## Quickstart

Currently only the `info` command is wired up. Build from source:

```sh
git clone https://github.com/11ways/dpub
cd dpub
cargo build --release
./target/release/dpub info /path/to/daisy/ncc.html
```

Example output for a real Vlaams DAISY 2.02 audiobook:

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

## Roadmap

| Milestone | Scope |
| --- | --- |
| **M0.5** | `dpub info` — read NCC metadata and nav summary. ✅ |
| **M1** | Full DAISY parser: NCC + master.smil + per-section SMIL + audio metadata; structural round-trip. ✅ |
| **M2** | Minimal EPUB 3 writer (audio-only with Media Overlays). EPUBCheck-clean. |
| **M3** | End-to-end: `dpub convert <ncc.html> -o out.epub`. |
| **M4** | Built-in validation (EPUBCheck + ACE) — `dpub validate`. |
| **M5** | Audio recompression (MP3 → Opus). |
| **M6** | Whisper transcription for audio-only books. |
| **M7** | WASM build for browser-based conversion. |
| **M8** | 1.0 release: macOS / Linux / Windows binaries. |

## Project layout

```
dpub/
├── crates/
│   ├── dpub-core/   # DAISY 2.02 model + parser
│   └── dpub-cli/    # `dpub` binary
└── ...
```

More crates land as later milestones come online (`epub3-writer`, `dpub-validate`, `dpub-audio`, `dpub-whisper`, `dpub-wasm`).

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
