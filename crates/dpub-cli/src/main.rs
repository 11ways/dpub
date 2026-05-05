use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use dpub_core::{Book, NavItem};

#[derive(Parser)]
#[command(name = "dpub", version, about = "DAISY 2.02 → EPUB 3 toolkit")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print metadata and structure of a DAISY 2.02 publication.
    Info {
        /// Path to the publication's `ncc.html`.
        ncc: PathBuf,
    },
    /// Convert a DAISY 2.02 publication to an EPUB 3 file.
    Convert {
        /// Path to the publication's `ncc.html`.
        ncc: PathBuf,
        /// Output path for the resulting `.epub` file.
        #[arg(short, long)]
        output: PathBuf,
        /// Run EPUBCheck against the produced file and report any issues.
        #[arg(long)]
        validate: bool,
        /// Audio handling: keep originals or recompress to Opus.
        #[arg(long, value_enum, default_value_t = AudioOpt::Original)]
        audio: AudioOpt,
        /// Bitrate (kbit/s) when --audio=opus. Sensible range: 32–96 for speech.
        #[arg(long, default_value_t = dpub_audio::DEFAULT_OPUS_BITRATE_KBPS)]
        bitrate: u32,
        /// Transcribe audio with local Whisper (e.g. `nl`, `en`). Requires
        /// `--whisper-model`. The text gets injected into each section's
        /// content document as a flat list of paragraphs.
        #[arg(long)]
        transcribe: Option<String>,
        /// Path to a `ggml-*.bin` Whisper model file. Required with
        /// `--transcribe`.
        #[arg(long)]
        whisper_model: Option<PathBuf>,
    },
    /// Validate an existing EPUB 3 publication with EPUBCheck.
    Validate {
        /// Path to the `.epub` file to validate.
        epub: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum AudioOpt {
    /// Embed source MP3s unchanged.
    Original,
    /// Re-encode every audio file to Ogg/Opus (requires `ffmpeg` on PATH).
    Opus,
}

impl AudioOpt {
    fn into_format(self, bitrate_kbps: u32) -> dpub_convert::AudioFormat {
        match self {
            AudioOpt::Original => dpub_convert::AudioFormat::Original,
            AudioOpt::Opus => dpub_convert::AudioFormat::Opus { bitrate_kbps },
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Info { ncc } => cmd_info(&ncc),
        Command::Convert {
            ncc,
            output,
            validate,
            audio,
            bitrate,
            transcribe,
            whisper_model,
        } => cmd_convert(
            &ncc,
            &output,
            validate,
            audio,
            bitrate,
            transcribe,
            whisper_model,
        ),
        Command::Validate { epub } => cmd_validate(&epub),
    }
}

fn cmd_convert(
    ncc: &std::path::Path,
    output: &std::path::Path,
    validate: bool,
    audio: AudioOpt,
    bitrate_kbps: u32,
    transcribe: Option<String>,
    whisper_model: Option<PathBuf>,
) -> Result<()> {
    let book = Book::from_ncc(ncc).with_context(|| format!("loading {}", ncc.display()))?;
    println!("Converting {} → {}", ncc.display(), output.display());
    println!(
        "  {} sections, {} sync points, {} audio clips, total {}",
        book.master.references.len(),
        book.total_par_count(),
        book.total_audio_clip_count(),
        format_duration(book.total_audio_seconds()),
    );
    if matches!(audio, AudioOpt::Opus) {
        if !dpub_audio::ffmpeg_available() {
            anyhow::bail!(
                "ffmpeg is not on PATH; install it (e.g. `brew install ffmpeg`) and retry"
            );
        }
        println!("  Audio: Opus @ {bitrate_kbps} kbit/s (re-encoding)");
    }

    let transcribe_opts = match (transcribe, whisper_model) {
        (Some(language), Some(model_path)) => {
            if !model_path.is_file() {
                anyhow::bail!(
                    "Whisper model not found at {} (download from https://huggingface.co/ggerganov/whisper.cpp)",
                    model_path.display()
                );
            }
            println!(
                "  Transcribe: lang={language} model={}",
                model_path.display()
            );
            Some(dpub_convert::TranscribeOptions {
                model_path,
                language,
            })
        }
        (Some(_), None) => {
            anyhow::bail!("--transcribe requires --whisper-model");
        }
        (None, Some(_)) => {
            anyhow::bail!("--whisper-model requires --transcribe");
        }
        (None, None) => None,
    };

    let opts = dpub_convert::ConvertOptions {
        audio: audio.into_format(bitrate_kbps),
        transcribe: transcribe_opts,
    };
    let start = std::time::Instant::now();
    dpub_convert::convert_to_file(&book, output, &opts)
        .with_context(|| format!("writing {}", output.display()))?;
    let elapsed = start.elapsed();
    let bytes = std::fs::metadata(output).map_or(0, |m| m.len());
    // Audiobooks rarely exceed a few hundred GiB, so the precision loss in the
    // u64→f64 cast for the human-readable size readout is irrelevant.
    #[allow(clippy::cast_precision_loss)]
    let mib = bytes as f64 / 1_048_576.0;
    println!(
        "Wrote {} ({:.1} MiB) in {:.2}s",
        output.display(),
        mib,
        elapsed.as_secs_f64(),
    );

    if validate {
        println!();
        cmd_validate(output)?;
    }
    Ok(())
}

fn cmd_validate(epub: &std::path::Path) -> Result<()> {
    if !dpub_validate::epubcheck_available() {
        anyhow::bail!(
            "epubcheck is not on PATH; install it (e.g. `brew install epubcheck`) and retry"
        );
    }
    let report = dpub_validate::validate_epub(epub)
        .with_context(|| format!("validating {}", epub.display()))?;
    print_report(&report);
    if !report.is_clean() {
        anyhow::bail!("validation reported errors");
    }
    Ok(())
}

fn print_report(report: &dpub_validate::Report) {
    let Some(epubcheck) = &report.epubcheck else {
        println!("No validators ran.");
        return;
    };
    println!(
        "EPUBCheck {}: {} fatals / {} errors / {} warnings / {} usages",
        epubcheck.version.as_deref().unwrap_or("?"),
        epubcheck.summary.fatals,
        epubcheck.summary.errors,
        epubcheck.summary.warnings,
        epubcheck.summary.infos,
    );
    for issue in &epubcheck.issues {
        println!(
            "  [{sev}] {id} {loc}{msg}",
            sev = issue.severity.as_str(),
            id = issue.id.as_deref().unwrap_or("-"),
            loc = match &issue.location {
                Some(l) => format!("{l} — "),
                None => String::new(),
            },
            msg = issue.message,
        );
    }
}

fn cmd_info(ncc_path: &std::path::Path) -> Result<()> {
    let book =
        Book::from_ncc(ncc_path).with_context(|| format!("loading {}", ncc_path.display()))?;
    let m = book.metadata();

    println!("Title:         {}", m.title.as_deref().unwrap_or("—"));
    println!("Creator:       {}", m.creator.as_deref().unwrap_or("—"));
    println!("Publisher:     {}", m.publisher.as_deref().unwrap_or("—"));
    println!("Date:          {}", m.date.as_deref().unwrap_or("—"));
    println!("Language:      {}", m.language.as_deref().unwrap_or("—"));
    println!("Identifier:    {}", m.identifier.as_deref().unwrap_or("—"));
    println!("Format:        {}", m.format.as_deref().unwrap_or("—"));
    println!(
        "Multimedia:    {}",
        m.multimedia_type.as_deref().unwrap_or("—")
    );
    println!("Narrator:      {}", m.narrator.as_deref().unwrap_or("—"));
    println!("Total time:    {}", m.total_time.as_deref().unwrap_or("—"));

    let headings = book.ncc.headings().count();
    let pages = book.ncc.pages().count();
    let by_level: std::collections::BTreeMap<u8, usize> =
        book.ncc
            .headings()
            .fold(std::collections::BTreeMap::new(), |mut acc, h| {
                *acc.entry(h.level).or_default() += 1;
                acc
            });

    println!();
    println!("Navigation:");
    println!("  Headings:    {headings} ({})", format_levels(&by_level));
    println!("  Pages:       {pages}");

    println!();
    println!("SMIL:");
    println!("  Sections:    {}", book.master.references.len());
    println!("  Synch points: {}", book.total_par_count());
    println!("  Audio clips: {}", book.total_audio_clip_count());
    println!(
        "  Audio total: {}",
        format_duration(book.total_audio_seconds())
    );
    println!("  Audio files: {}", book.audio_files().len());

    if !m.other.is_empty() {
        println!();
        println!("Other metadata:");
        for (k, v) in &m.other {
            println!("  {k}: {v}");
        }
    }

    if let Some(NavItem::Heading(first)) = book.ncc.nav.first() {
        println!();
        println!("First heading: \"{}\" → {}", first.text, first.href);
    }

    Ok(())
}

fn format_duration(seconds: f64) -> String {
    // Cap at one year (~31.5M seconds), well past any realistic talking book.
    // The cast is safe in this range and avoids float-to-int UB / overflow.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let total = seconds.clamp(0.0, 31_536_000.0).round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

fn format_levels(by_level: &std::collections::BTreeMap<u8, usize>) -> String {
    by_level
        .iter()
        .map(|(level, count)| format!("h{level}: {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}
