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
        /// Path to the publication's `ncc.html`, or the directory containing it.
        ncc: PathBuf,
    },
    /// Convert a DAISY 2.02 publication to an EPUB 3 file.
    Convert {
        /// Path to the publication's `ncc.html`, or the directory containing it.
        ncc: PathBuf,
        /// Output path for the resulting `.epub` file.
        #[arg(short, long)]
        output: PathBuf,
        /// Run EPUBCheck against the produced file and report any issues.
        #[arg(long)]
        validate: bool,
        /// Run DAISY ACE accessibility checks against the produced file and
        /// report any issues. Requires `ace` on PATH (`npm install -g @daisy/ace`).
        #[arg(long)]
        a11y: bool,
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
        /// Emit one `<p>` per Whisper segment instead of merging into
        /// prose-shaped paragraphs. Useful for debugging the raw model
        /// output; not recommended for distribution.
        #[arg(long)]
        no_text_cleanup: bool,
        /// Skip per-word Media Overlay sync. Word-level sync (the
        /// default for transcribed books) drives karaoke-style
        /// highlight-along-with-audio in compatible reading systems
        /// (Thorium, Readium). Pass this flag to fall back to
        /// per-paragraph sync — produces a smaller SMIL at the cost
        /// of a coarser reading experience.
        #[arg(long)]
        no_word_sync: bool,
        /// Path to a JPEG or PNG image to embed as the EPUB cover.
        #[arg(long, value_name = "PATH", conflicts_with = "auto_cover")]
        cover: Option<PathBuf>,
        /// Best-effort cover lookup via Open Library using the book's
        /// title, author, and identifier. Opt-in: it sends those bits
        /// of metadata to a third party (Open Library). A miss is
        /// silent — the book ships without a cover.
        #[arg(long)]
        auto_cover: bool,
        /// Free-text rights statement to stamp into the EPUB's
        /// `<dc:rights>` field. Overrides any rights string in the
        /// source DAISY metadata.
        #[arg(long, value_name = "TEXT")]
        rights: Option<String>,
    },
    /// Validate an existing EPUB 3 publication with EPUBCheck.
    Validate {
        /// Path to the `.epub` file to validate.
        epub: PathBuf,
        /// Emit the structured report as JSON on stdout instead of the
        /// human-readable summary.
        #[arg(long)]
        json: bool,
    },
    /// Run accessibility checks (DAISY ACE) on an existing EPUB 3 publication.
    /// Requires `ace` on PATH (`npm install -g @daisy/ace`).
    A11y {
        /// Path to the `.epub` file to check.
        epub: PathBuf,
        /// Emit the structured report as JSON on stdout instead of the
        /// human-readable summary.
        #[arg(long)]
        json: bool,
    },
    /// Convert every DAISY 2.02 book under `<input>` to EPUB 3, in
    /// parallel. A "book" is any directory containing an `ncc.html`.
    /// Writes a JSON summary to stdout when finished. Per-book errors
    /// are recorded in the summary, not raised — one bad book never
    /// halts the queue.
    Batch {
        /// Directory to scan for DAISY 2.02 books (recursively).
        input: PathBuf,
        /// Directory to write `.epub` files to. Created if missing.
        #[arg(short, long)]
        output: PathBuf,
        /// Number of conversions to run in parallel. `0` (default) lets
        /// rayon pick (typically the CPU count).
        #[arg(short, long, default_value_t = 0)]
        jobs: usize,
        /// Audio handling: keep originals or recompress to Opus.
        #[arg(long, value_enum, default_value_t = AudioOpt::Original)]
        audio: AudioOpt,
        /// Bitrate (kbit/s) when --audio=opus. Sensible range: 32–96 for speech.
        #[arg(long, default_value_t = dpub_audio::DEFAULT_OPUS_BITRATE_KBPS)]
        bitrate: u32,
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
            a11y,
            audio,
            bitrate,
            transcribe,
            whisper_model,
            no_text_cleanup,
            no_word_sync,
            cover,
            auto_cover,
            rights,
        } => cmd_convert(
            &ncc,
            &output,
            validate,
            a11y,
            audio,
            bitrate,
            transcribe,
            whisper_model,
            no_text_cleanup,
            no_word_sync,
            cover,
            auto_cover,
            rights,
        ),
        Command::Validate { epub, json } => cmd_validate(&epub, json),
        Command::A11y { epub, json } => cmd_a11y(&epub, json),
        Command::Batch {
            input,
            output,
            jobs,
            audio,
            bitrate,
        } => cmd_batch(&input, &output, jobs, audio, bitrate),
    }
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn cmd_convert(
    ncc: &std::path::Path,
    output: &std::path::Path,
    validate: bool,
    a11y: bool,
    audio: AudioOpt,
    bitrate_kbps: u32,
    transcribe: Option<String>,
    whisper_model: Option<PathBuf>,
    no_text_cleanup: bool,
    no_word_sync: bool,
    cover: Option<PathBuf>,
    auto_cover: bool,
    rights: Option<String>,
) -> Result<()> {
    let ncc = resolve_ncc_path(ncc)?;
    let book = Book::from_ncc(&ncc).with_context(|| format!("loading {}", ncc.display()))?;
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

    if let Some(path) = &cover {
        if !path.is_file() {
            anyhow::bail!("cover image not found at {}", path.display());
        }
        println!("  Cover: {}", path.display());
    } else if auto_cover {
        println!("  Cover: best-effort lookup via Open Library");
    }

    let opts = dpub_convert::ConvertOptions {
        audio: audio.into_format(bitrate_kbps),
        transcribe: transcribe_opts,
        raw_transcript_segments: no_text_cleanup,
        cover,
        auto_cover,
        rights,
        no_word_sync,
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
        cmd_validate(output, false)?;
    }
    if a11y {
        println!();
        cmd_a11y(output, false)?;
    }
    Ok(())
}

fn cmd_validate(epub: &std::path::Path, json: bool) -> Result<()> {
    if !dpub_validate::epubcheck_available() {
        anyhow::bail!(
            "epubcheck is not on PATH; install it (e.g. `brew install epubcheck`) and retry"
        );
    }
    let report = dpub_validate::Report {
        epubcheck: Some(
            dpub_validate::run_epubcheck(epub)
                .with_context(|| format!("validating {}", epub.display()))?,
        ),
        ace: None,
    };
    emit_report(&report, json)?;
    if !report.is_clean() {
        anyhow::bail!("validation reported errors");
    }
    Ok(())
}

fn cmd_a11y(epub: &std::path::Path, json: bool) -> Result<()> {
    if !dpub_validate::ace_available() {
        anyhow::bail!(
            "ace is not on PATH; install it with `npm install -g @daisy/ace` and retry"
        );
    }
    let report = dpub_validate::Report {
        epubcheck: None,
        ace: Some(
            dpub_validate::run_ace(epub)
                .with_context(|| format!("running ace on {}", epub.display()))?,
        ),
    };
    emit_report(&report, json)?;
    if !report.is_clean() {
        anyhow::bail!("accessibility checker reported errors");
    }
    Ok(())
}

/// Either print the human-readable summary, or serialise the report to
/// stdout as pretty JSON. The JSON shape is the `Report` struct from
/// `dpub-validate`; field names are stable as part of the 1.0 contract.
fn emit_report(report: &dpub_validate::Report, json: bool) -> Result<()> {
    if json {
        let s = serde_json::to_string_pretty(report).context("serialise report")?;
        println!("{s}");
    } else {
        print_report(report);
    }
    Ok(())
}

fn print_report(report: &dpub_validate::Report) {
    if report.epubcheck.is_none() && report.ace.is_none() {
        println!("No validators ran.");
        return;
    }
    if let Some(b) = &report.epubcheck {
        print_backend("EPUBCheck", b);
    }
    if let Some(b) = &report.ace {
        print_backend("ACE", b);
    }
}

fn print_backend(label: &str, report: &dpub_validate::BackendReport) {
    println!(
        "{label} {}: {} fatals / {} errors / {} warnings / {} usages",
        report.version.as_deref().unwrap_or("?"),
        report.summary.fatals,
        report.summary.errors,
        report.summary.warnings,
        report.summary.infos,
    );
    for issue in &report.issues {
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
    let ncc_path = resolve_ncc_path(ncc_path)?;
    let book =
        Book::from_ncc(&ncc_path).with_context(|| format!("loading {}", ncc_path.display()))?;
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

#[derive(serde::Serialize)]
struct BatchSummary {
    /// Number of books discovered under `input`.
    total: usize,
    succeeded: usize,
    failed: usize,
    /// Wallclock seconds for the whole batch (includes parallelism).
    wallclock_seconds: f64,
    books: Vec<BatchEntry>,
}

#[derive(serde::Serialize)]
struct BatchEntry {
    /// Path to the source `ncc.html`.
    input: String,
    /// Path to the produced `.epub`. Absent when `status == "error"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    /// `"ok"` or `"error"`.
    status: &'static str,
    duration_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_mib: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn cmd_batch(
    input: &std::path::Path,
    output: &std::path::Path,
    jobs: usize,
    audio: AudioOpt,
    bitrate_kbps: u32,
) -> Result<()> {
    use rayon::prelude::*;

    if !input.is_dir() {
        anyhow::bail!("batch input {} is not a directory", input.display());
    }
    std::fs::create_dir_all(output)
        .with_context(|| format!("creating output dir {}", output.display()))?;
    if matches!(audio, AudioOpt::Opus) && !dpub_audio::ffmpeg_available() {
        anyhow::bail!("ffmpeg is not on PATH; install it (e.g. `brew install ffmpeg`) and retry");
    }

    // Walk for ncc.html (case-insensitive). Each match identifies one book.
    let books: Vec<PathBuf> = walkdir::WalkDir::new(input)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| {
            e.file_type().is_file()
                && e.file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("ncc.html")
        })
        .map(walkdir::DirEntry::into_path)
        .collect();

    if books.is_empty() {
        eprintln!(
            "no DAISY books (no `ncc.html`) found under {}",
            input.display()
        );
        let summary = BatchSummary {
            total: 0,
            succeeded: 0,
            failed: 0,
            wallclock_seconds: 0.0,
            books: vec![],
        };
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }

    eprintln!("Batch: {} book(s) under {}", books.len(), input.display());

    // Configure the rayon pool size.
    if jobs > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build_global()
            .ok(); // ignore "already initialised"
    }

    let opts = dpub_convert::ConvertOptions {
        audio: audio.into_format(bitrate_kbps),
        transcribe: None,
        raw_transcript_segments: false,
        cover: None,
        auto_cover: false,
        rights: None,
        no_word_sync: false,
    };
    let start = std::time::Instant::now();
    let entries: Vec<BatchEntry> = books
        .par_iter()
        .map(|ncc_path| {
            let book_start = std::time::Instant::now();
            let stem = ncc_path
                .parent()
                .and_then(|p| p.file_name())
                .map_or_else(|| "book".to_owned(), |s| s.to_string_lossy().into_owned());
            let epub_path = output.join(format!("{stem}.epub"));
            let result = (|| -> std::result::Result<(), anyhow::Error> {
                let book = dpub_core::Book::from_ncc(ncc_path)?;
                dpub_convert::convert_to_file(&book, &epub_path, &opts)?;
                Ok(())
            })();
            let duration = book_start.elapsed().as_secs_f64();
            match result {
                Ok(()) => {
                    let size_bytes = std::fs::metadata(&epub_path).map_or(0, |m| m.len());
                    #[allow(clippy::cast_precision_loss)]
                    let mib = size_bytes as f64 / 1_048_576.0;
                    eprintln!("  ✓ {} → {} ({mib:.1} MiB, {duration:.1}s)", ncc_path.display(), epub_path.display());
                    BatchEntry {
                        input: ncc_path.display().to_string(),
                        output: Some(epub_path.display().to_string()),
                        status: "ok",
                        duration_seconds: duration,
                        size_mib: Some(mib),
                        error: None,
                    }
                }
                Err(err) => {
                    eprintln!("  ✗ {}: {err:#}", ncc_path.display());
                    BatchEntry {
                        input: ncc_path.display().to_string(),
                        output: None,
                        status: "error",
                        duration_seconds: duration,
                        size_mib: None,
                        error: Some(format!("{err:#}")),
                    }
                }
            }
        })
        .collect();

    let succeeded = entries.iter().filter(|e| e.status == "ok").count();
    let failed = entries.len() - succeeded;
    let summary = BatchSummary {
        total: entries.len(),
        succeeded,
        failed,
        wallclock_seconds: start.elapsed().as_secs_f64(),
        books: entries,
    };
    println!("{}", serde_json::to_string_pretty(&summary)?);
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Accept either an `ncc.html` file directly, or a directory containing one.
/// Tries the spec-mandated lowercase name first; falls back to a
/// case-insensitive scan so legacy books with `NCC.HTML` still resolve.
fn resolve_ncc_path(input: &std::path::Path) -> Result<PathBuf> {
    let meta = std::fs::metadata(input)
        .with_context(|| format!("reading {}", input.display()))?;
    if meta.is_file() {
        return Ok(input.to_path_buf());
    }
    if !meta.is_dir() {
        anyhow::bail!("{} is neither a file nor a directory", input.display());
    }
    let direct = input.join("ncc.html");
    if direct.is_file() {
        return Ok(direct);
    }
    for entry in std::fs::read_dir(input)
        .with_context(|| format!("reading directory {}", input.display()))?
    {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case("ncc.html")
            && entry.file_type().is_ok_and(|t| t.is_file())
        {
            return Ok(entry.path());
        }
    }
    anyhow::bail!(
        "no `ncc.html` found in directory {} — is this a DAISY 2.02 publication?",
        input.display()
    );
}

#[cfg(test)]
mod tests {
    use super::resolve_ncc_path;
    use std::fs;

    #[test]
    fn resolves_a_file_path_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ncc.html");
        fs::write(&path, "<html></html>").unwrap();
        assert_eq!(resolve_ncc_path(&path).unwrap(), path);
    }

    #[test]
    fn resolves_directory_to_ncc_html() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ncc.html");
        fs::write(&path, "<html></html>").unwrap();
        assert_eq!(resolve_ncc_path(dir.path()).unwrap(), path);
    }

    #[test]
    fn resolves_directory_with_uppercase_ncc() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("NCC.HTML");
        fs::write(&path, "<html></html>").unwrap();
        let resolved = resolve_ncc_path(dir.path()).unwrap();
        // On case-insensitive filesystems (macOS default) the `ncc.html` probe
        // succeeds and returns that exact form; on case-sensitive filesystems
        // (Linux CI) the directory scan returns the literal `NCC.HTML`. Either
        // is correct as long as it points at a real file in the dir.
        assert!(resolved.is_file());
        assert_eq!(resolved.parent(), Some(dir.path()));
        assert!(
            resolved
                .file_name()
                .unwrap()
                .to_string_lossy()
                .eq_ignore_ascii_case("ncc.html")
        );
    }

    #[test]
    fn errors_on_directory_without_ncc() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("master.smil"), "").unwrap();
        let err = resolve_ncc_path(dir.path()).unwrap_err();
        assert!(err.to_string().contains("no `ncc.html`"));
    }

    #[test]
    fn errors_on_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(resolve_ncc_path(&missing).is_err());
    }
}
