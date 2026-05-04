## Summary

<!-- One or two sentences. Why this change exists. -->

## Changes

<!-- Bullet points. -->

## Linked issue

<!-- Closes #N, Refs #N. -->

## How to verify

<!-- Commands the reviewer can run, or output to inspect. -->

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

## Checklist

- [ ] `cargo fmt` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] CHANGELOG entry added (under `[Unreleased]`) if user-visible
