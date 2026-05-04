# Contributing to dpub

Thanks for your interest! dpub is in early development. Bug reports, design feedback, and small focused PRs are all welcome.

## Ground rules

- Be kind. The [`Code of Conduct`](CODE_OF_CONDUCT.md) applies.
- Open an issue **before** starting non-trivial work — we may already be working on it, or know of a constraint that would change your design.
- One concern per PR. A PR that fixes a bug should not also restructure neighbouring code.
- Accessibility is the whole point of this project. If a change makes output less accessible, even by accident, it's a regression.

## Development setup

You need a recent stable Rust toolchain (1.85+, Edition 2024).

```sh
git clone https://github.com/11ways/dpub
cd dpub
cargo build
cargo test
```

Optional but recommended for local validation parity with CI:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## Project structure

- `crates/dpub-core/` — DAISY 2.02 in-memory model and parser. No I/O beyond reading input files.
- `crates/dpub-cli/` — the `dpub` binary. User-facing concerns only; logic lives in core / domain crates.

Future crates (`epub3-writer`, `dpub-validate`, `dpub-audio`, `dpub-whisper`, `dpub-wasm`) are introduced milestone by milestone.

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

## Licensing of contributions

By submitting a contribution to this project, you agree to license your contribution under the same terms as dpub itself: dual MIT / Apache-2.0. No CLA is required.
