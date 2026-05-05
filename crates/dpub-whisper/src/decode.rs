//! Decode any audio format `symphonia` understands (MP3, AAC, Vorbis, …)
//! into the floating-point mono 16 kHz PCM that whisper.cpp wants.
//!
//! Steps:
//!
//! 1. Symphonia probes the container, picks the best audio track, and
//!    returns a stream of decoded samples (interleaved if multi-channel,
//!    typed as `i16` / `i32` / `f32` depending on the codec).
//! 2. We collapse to mono by averaging channels.
//! 3. We resample to 16 kHz with `rubato`'s `SincFixedIn` (high quality,
//!    polyphase) using SincInterpolationParameters tuned for speech.
//!
//! No process spawn, no ffmpeg — pure-Rust pipeline.

#![allow(
    clippy::similar_names,         // `mono_at_source` / `source_rate` etc. are local pairs
    clippy::cast_possible_truncation, // sample-width conversions are inherently lossy
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::fs::File;
use std::path::Path;

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::{DecodeError, Error};

const TARGET_RATE: u32 = 16_000;

pub(crate) fn decode_to_mono_16khz(path: &Path) -> crate::Result<Vec<f32>> {
    decode_inner(path).map_err(|source| Error::Decode {
        path: path.to_path_buf(),
        source,
    })
}

fn decode_inner(path: &Path) -> Result<Vec<f32>, DecodeError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(
        Box::new(file),
        symphonia::core::io::MediaSourceStreamOptions::default(),
    );

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;
    let source_rate = track.codec_params.sample_rate.unwrap_or(TARGET_RATE);
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    // Drain the file into a single mono f32 buffer at the source rate.
    let mut mono_at_source: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(other) => return Err(DecodeError::Symphonia(other)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => append_mono(&buf, &mut mono_at_source),
            // Skip bad packets (real-world DAISY MP3s sometimes have a
            // truncated last frame). The transcript loses a few ms; better
            // than failing the whole file.
            Err(SymphoniaError::DecodeError(_)) => {}
            Err(e) => return Err(DecodeError::Symphonia(e)),
        }
    }

    if source_rate == TARGET_RATE {
        return Ok(mono_at_source);
    }
    resample_to_16khz(&mono_at_source, source_rate)
}

/// Average all channels of a decoded buffer into a single mono `f32` track
/// and append to `out`.
fn append_mono(buf: &AudioBufferRef<'_>, out: &mut Vec<f32>) {
    use symphonia::core::audio::AudioBufferRef::{F32, F64, S8, S16, S24, S32, U8, U16, U24, U32};
    macro_rules! collapse {
        ($spec:expr, $convert:expr) => {{
            let buf = $spec;
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            out.reserve(frames);
            if channels == 1 {
                for &s in buf.chan(0) {
                    out.push($convert(s));
                }
            } else {
                let chans: Vec<&[_]> = (0..channels).map(|c| buf.chan(c)).collect();
                #[allow(clippy::cast_precision_loss)]
                let n = channels as f32;
                for i in 0..frames {
                    let mut sum = 0f32;
                    for ch in &chans {
                        sum += $convert(ch[i]);
                    }
                    out.push(sum / n);
                }
            }
        }};
    }

    match buf {
        F32(b) => collapse!(b, |s: f32| s),
        F64(b) => collapse!(b, |s: f64| s as f32),
        S8(b) => collapse!(b, |s: i8| f32::from(s) / 128.0),
        S16(b) => collapse!(b, |s: i16| f32::from(s) / f32::from(i16::MAX)),
        S24(b) => collapse!(b, |s: symphonia::core::sample::i24| {
            #[allow(clippy::cast_precision_loss)]
            let v = s.inner() as f32 / 8_388_608.0;
            v
        }),
        #[allow(clippy::cast_precision_loss)]
        S32(b) => collapse!(b, |s: i32| s as f32 / 2_147_483_648.0),
        U8(b) => collapse!(b, |s: u8| (f32::from(s) - 128.0) / 128.0),
        U16(b) => collapse!(b, |s: u16| (f32::from(s) - 32_768.0) / 32_768.0),
        U24(b) => collapse!(b, |s: symphonia::core::sample::u24| {
            #[allow(clippy::cast_precision_loss)]
            let v = (s.inner() as f32 - 8_388_608.0) / 8_388_608.0;
            v
        }),
        #[allow(clippy::cast_precision_loss)]
        U32(b) => collapse!(b, |s: u32| (s as f32 - 2_147_483_648.0) / 2_147_483_648.0),
    }
}

fn resample_to_16khz(input: &[f32], source_rate: u32) -> Result<Vec<f32>, DecodeError> {
    // `SincFixedIn` works in fixed-size chunks. We pick a chunk size that's
    // friendly to typical MP3 frame sizes and the SincInterpolation cost.
    const CHUNK: usize = 4096;

    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        oversampling_factor: 128,
        interpolation: SincInterpolationType::Linear,
        window: WindowFunction::BlackmanHarris2,
    };
    let ratio = f64::from(TARGET_RATE) / f64::from(source_rate);
    let mut resampler = SincFixedIn::<f32>::new(ratio, 1.0, params, CHUNK, 1)?;

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let cap = ((input.len() as f64) * ratio + 1.0) as usize;
    let mut out = Vec::with_capacity(cap);
    let mut chunk = vec![0f32; CHUNK];
    let mut consumed = 0;

    while consumed + CHUNK <= input.len() {
        chunk.copy_from_slice(&input[consumed..consumed + CHUNK]);
        let resampled = resampler.process(&[&chunk], None)?;
        out.extend_from_slice(&resampled[0]);
        consumed += CHUNK;
    }

    // Pad remainder with zeros; whisper handles trailing silence gracefully.
    if consumed < input.len() {
        chunk[..input.len() - consumed].copy_from_slice(&input[consumed..]);
        for x in &mut chunk[input.len() - consumed..] {
            *x = 0.0;
        }
        let resampled = resampler.process(&[&chunk], None)?;
        out.extend_from_slice(&resampled[0]);
    }

    Ok(out)
}
