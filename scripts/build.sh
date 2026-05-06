#!/usr/bin/env bash
# Host-aware release build for dpub.
#
# Picks the right whisper.cpp acceleration feature for the host:
#   - Apple Silicon → --features metal  (~5–10× faster Whisper)
#   - Linux + nvcc  → --features cuda
#   - Everything else → CPU-only
#
# Power users who want different feature flags should call
#   `cargo build --release -p dpub-cli ...` directly.

set -euo pipefail

# Pre-flight: cmake is required to build dpub-whisper (compiles
# whisper.cpp from source via whisper-rs-sys).
if ! command -v cmake >/dev/null 2>&1; then
  echo >&2 "error: cmake is required to build dpub (whisper-rs-sys compiles whisper.cpp)"
  case "$(uname -s)" in
    Darwin) echo >&2 "       install: brew install cmake" ;;
    Linux)  echo >&2 "       install: sudo apt-get install -y cmake  (or: sudo dnf install -y cmake)" ;;
  esac
  exit 1
fi

features=()
case "$(uname -s)/$(uname -m)" in
  Darwin/arm64)
    features+=("metal")
    echo "Detected Apple Silicon — building with Metal Whisper acceleration."
    ;;
  Darwin/x86_64)
    echo "Detected Intel macOS — building CPU-only (Metal needs Apple Silicon)."
    ;;
  Linux/x86_64)
    if command -v nvcc >/dev/null 2>&1; then
      features+=("cuda")
      echo "Detected NVIDIA toolkit — building with CUDA Whisper acceleration."
    else
      echo "Detected Linux x86_64 — building CPU-only (install CUDA toolkit for nvcc-detected GPU build)."
    fi
    ;;
  *)
    echo "Unknown host $(uname -s)/$(uname -m) — building CPU-only."
    ;;
esac

set -x
if [ ${#features[@]} -gt 0 ]; then
  exec cargo build --release -p dpub-cli --features "${features[*]}"
else
  exec cargo build --release -p dpub-cli
fi
