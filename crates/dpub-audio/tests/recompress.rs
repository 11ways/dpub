//! `ffmpeg`-backed Opus re-encoding smoke test.
//!
//! Skipped (silently) if `ffmpeg` is not on `PATH` so the test stays
//! green on machines without it.

use std::fs::File;
use std::io::{Read, Write};

const TINY_MP3: &[u8] = include_bytes!("../../epub3-writer/tests/fixtures/tiny.mp3");

#[test]
fn round_trip_tiny_mp3_to_opus() {
    if !dpub_audio::ffmpeg_available() {
        eprintln!("ffmpeg not on PATH — skipping");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let mp3 = dir.path().join("tiny.mp3");
    File::create(&mp3)
        .and_then(|mut f| f.write_all(TINY_MP3))
        .expect("write mp3 fixture");

    let opus = dir.path().join("tiny.opus");
    dpub_audio::recompress_to_opus(&mp3, &opus, 32).expect("recompress");

    let bytes = std::fs::metadata(&opus).expect("stat").len();
    assert!(bytes > 0, "opus output is empty");

    // Ogg streams start with the magic four bytes "OggS".
    let mut head = [0u8; 4];
    File::open(&opus)
        .and_then(|mut f| f.read_exact(&mut head))
        .expect("read magic");
    assert_eq!(&head, b"OggS", "output is not an Ogg stream");
}
