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

fn format_levels(by_level: &std::collections::BTreeMap<u8, usize>) -> String {
    by_level
        .iter()
        .map(|(level, count)| format!("h{level}: {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}
