use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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
    },
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
        Command::Convert { ncc, output } => cmd_convert(&ncc, &output),
    }
}

fn cmd_convert(ncc: &std::path::Path, output: &std::path::Path) -> Result<()> {
    let book = Book::from_ncc(ncc).with_context(|| format!("loading {}", ncc.display()))?;
    println!("Converting {} → {}", ncc.display(), output.display());
    println!(
        "  {} sections, {} sync points, {} audio clips, total {}",
        book.master.references.len(),
        book.total_par_count(),
        book.total_audio_clip_count(),
        format_duration(book.total_audio_seconds()),
    );
    let start = std::time::Instant::now();
    dpub_convert::convert_to_file(&book, output)
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
    Ok(())
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
