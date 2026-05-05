# Contributing to dpub

Thanks for your interest! dpub is in early development. Bug reports, design feedback, and small focused PRs are all welcome.

## Ground rules

- Be kind. The [`Code of Conduct`](CODE_OF_CONDUCT.md) applies.
- Open an issue **before** starting non-trivial work — we may already be working on it, or know of a constraint that would change your design.
- One concern per PR. A PR that fixes a bug should not also restructure neighbouring code.
- Accessibility is the whole point of this project. If a change makes output less accessible, even by accident, it's a regression.

## Development setup

You need a recent stable Rust toolchain (MSRV 1.88, Edition 2024) and `cmake` for the bundled `dpub-whisper` crate (the `whisper-rs-sys` dep compiles whisper.cpp from source).

```sh
git clone https://github.com/11ways/dpub
cd dpub
cargo build
cargo test
```

Several optional dev tools turn on additional integration tests; see the table in [`README.md`](README.md#local-development).

Optional but recommended for local validation parity with CI:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## Project structure

- `crates/dpub-core/` — DAISY 2.02 in-memory model and parser. No I/O beyond reading input files.
- `crates/epub3-writer/` — typed EPUB 3 model and ZIP serialiser (Media Overlays, cover image).
- `crates/dpub-convert/` — drives the DAISY 2.02 → EPUB 3 pipeline.
- `crates/dpub-validate/` — EPUBCheck and ACE wrappers, structured `Report`.
- `crates/dpub-audio/` — ffmpeg-driven MP3 → Opus re-encoder.
- `crates/dpub-whisper/` — local Whisper transcription via whisper.cpp FFI.
- `crates/dpub-meta/` — external metadata lookup (currently Open Library covers).
- `crates/dpub-util/` — small shared helpers (XML escaping).
- `crates/dpub-cli/` — the `dpub` binary. User-facing concerns only; logic lives in the domain crates.

`dpub-wasm` is the next planned crate (M7).

## Testing

Unit tests live alongside the code. Integration tests against real DAISY books should go in a `tests/fixtures/` directory once we add one — the goal is to have at least one full-text-full-audio and one audio-only book under test at all times.

If you have a real DAISY book that exposes an edge case the parser mishandles, please open an issue. We may not be able to redistribute the book, but the structural details (NCC metadata, SMIL excerpts) are usually enough to write a focused test.

## Coding style

- `cargo fmt`. CI fails otherwise.
- `cargo clippy --all-targets -- -D warnings`. CI fails otherwise.
- Prefer small functions, but don't split for the sake of it.
- Comments explain **why**, not what. The code shows what.
- Avoid unsafe (forbidden at the crate level).

## Commit messages

Conventional-ish, but we don't enforce a strict format. Aim for:

```
short imperative subject (≤72 chars)

Optional body explaining the *why* if it isn't obvious from the diff.
Reference issues with `Refs #N` or `Closes #N`.
```

## Releasing

Releases are cut by tag. The [`.github/workflows/release.yml`](.github/workflows/release.yml) workflow fires on any `v*` tag push and builds the `dpub` binary for Linux (x86_64), macOS (arm64, with Metal Whisper acceleration), and Windows (x86_64); it then uploads each as an asset on a GitHub Release of the same name.

To cut a release:

1. Bump `version` in the workspace `Cargo.toml` (e.g. `0.1.0-dev` → `0.5.0`).
2. Update `CHANGELOG.md`: rename the `[Unreleased]` section to the new version with today's date, and add a fresh empty `[Unreleased]` block above it.
3. Commit on `main` (via PR) and tag: `git tag v0.5.0 && git push origin v0.5.0`.
4. Wait for the workflow to finish; verify the artifacts attached to the release page work on each platform.

Binaries from this workflow are **not signed**. macOS code signing + notarisation requires Apple Developer credentials in repo secrets, which is deferred until the project commits to that maintenance burden.

## Licensing of contributions

By submitting a contribution to this project, you agree to license your contribution under the same terms as dpub itself: dual MIT / Apache-2.0. No CLA is required.
