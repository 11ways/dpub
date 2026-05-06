//! Platform-aware best-effort installer for dpub's runtime tools.
//!
//! Invoked from `dpub doctor --install`. For each missing tool the
//! report flags, look up the right command for the host, prompt the
//! user (skip if `--yes`), then run it. Skips tools we can't safely
//! handle on a given platform (e.g. epubcheck on Linux) and prints
//! a URL the user can follow instead.
//!
//! Never auto-installs Java — too much variability in users' JVM
//! setups. Java is a hint, not an action.

use std::io::Write;
use std::process::Command;

use anyhow::{Context, Result};

use crate::doctor::{Report, Status, Tool};

#[derive(Debug, Clone)]
struct InstallStep {
    /// Human-readable label of the tool we're installing for.
    tool_label: String,
    /// Argv to run. First element is the binary name.
    argv: Vec<String>,
    /// Whether the command needs to be run via `sudo` (Linux package
    /// managers typically do).
    needs_sudo: bool,
}

/// Run the installer for every missing tool the report flags. Returns
/// `Ok(())` even when individual installs fail — the user sees the
/// errors inline and can rerun `doctor` to see the new state.
pub fn run_install(report: &Report, yes: bool) -> Result<()> {
    let plan = plan_install(report);
    if plan.is_empty() {
        println!("Nothing to install — `dpub doctor` is already green.");
        return Ok(());
    }
    println!("Will run the following commands to install missing tools:");
    println!();
    for step in &plan {
        let prefix = if step.needs_sudo { "sudo " } else { "" };
        println!("  {} → {}{}", step.tool_label, prefix, step.argv.join(" "));
    }
    println!();

    if !yes && !confirm("Proceed?")? {
        println!("Aborted.");
        return Ok(());
    }

    for step in plan {
        println!();
        let prefix = if step.needs_sudo { "sudo " } else { "" };
        println!("==> {} {}{}", step.tool_label, prefix, step.argv.join(" "));
        let status = if step.needs_sudo {
            Command::new("sudo")
                .args(&step.argv)
                .status()
                .with_context(|| format!("spawning sudo {}", step.argv.join(" ")))?
        } else {
            Command::new(&step.argv[0])
                .args(&step.argv[1..])
                .status()
                .with_context(|| format!("spawning {}", step.argv.join(" ")))?
        };
        if !status.success() {
            eprintln!(
                "  ✗ {} failed (exit code {:?}). Continuing with remaining steps.",
                step.tool_label,
                status.code(),
            );
        }
    }
    Ok(())
}

fn plan_install(report: &Report) -> Vec<InstallStep> {
    let mut out = Vec::new();
    for tool in &report.tools {
        if tool.status == Status::Ok {
            continue;
        }
        if let Some(step) = step_for(tool) {
            out.push(step);
        }
    }
    out
}

fn step_for(tool: &Tool) -> Option<InstallStep> {
    let label = tool.label.to_owned();
    match tool.key {
        "epubcheck" => epubcheck_step(label),
        "ace" => ace_step(label),
        "ffmpeg" => ffmpeg_step(label),
        // Anything else (notably "whisper-model", which is `dpub
        // setup` territory rather than an OS package) the doctor
        // report already points the user at the right command.
        _ => None,
    }
}

fn epubcheck_step(label: String) -> Option<InstallStep> {
    if cfg!(target_os = "macos") && which::which("brew").is_ok() {
        return Some(InstallStep {
            tool_label: label,
            argv: vec!["brew".into(), "install".into(), "epubcheck".into()],
            needs_sudo: false,
        });
    }
    eprintln!(
        "  • epubcheck has no straightforward package on this platform. \
         Download from https://github.com/w3c/epubcheck/releases (also needs Java 11)."
    );
    None
}

fn ace_step(label: String) -> Option<InstallStep> {
    if which::which("npm").is_ok() {
        return Some(InstallStep {
            tool_label: label,
            argv: vec![
                "npm".into(),
                "install".into(),
                "-g".into(),
                "@daisy/ace".into(),
            ],
            needs_sudo: !cfg!(target_os = "macos") && cfg!(target_os = "linux"),
        });
    }
    eprintln!(
        "  • npm not found. Install Node.js first, then run: npm install -g @daisy/ace"
    );
    None
}

fn ffmpeg_step(label: String) -> Option<InstallStep> {
    if cfg!(target_os = "macos") && which::which("brew").is_ok() {
        return Some(InstallStep {
            tool_label: label,
            argv: vec!["brew".into(), "install".into(), "ffmpeg".into()],
            needs_sudo: false,
        });
    }
    if cfg!(target_os = "linux") {
        if which::which("apt-get").is_ok() {
            return Some(InstallStep {
                tool_label: label,
                argv: vec![
                    "apt-get".into(),
                    "install".into(),
                    "-y".into(),
                    "ffmpeg".into(),
                ],
                needs_sudo: true,
            });
        }
        if which::which("dnf").is_ok() {
            return Some(InstallStep {
                tool_label: label,
                argv: vec![
                    "dnf".into(),
                    "install".into(),
                    "-y".into(),
                    "ffmpeg".into(),
                ],
                needs_sudo: true,
            });
        }
    }
    eprintln!(
        "  • No supported package manager detected for ffmpeg. \
         See https://ffmpeg.org/download.html"
    );
    None
}

/// Yes/no prompt on stderr; default = yes (empty answer accepted).
fn confirm(question: &str) -> Result<bool> {
    eprint!("{question} [Y/n] ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).context("read stdin")?;
    let answer = answer.trim().to_ascii_lowercase();
    Ok(answer.is_empty() || answer == "y" || answer == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::{Status, Tool};

    fn missing(key: &'static str, label: &'static str) -> Tool {
        Tool {
            key,
            label,
            status: Status::Missing,
            version: None,
            detail: None,
            install_hint: None,
        }
    }

    #[test]
    fn plan_skips_ok_tools() {
        let mut tool = missing("ffmpeg", "ffmpeg");
        tool.status = Status::Ok;
        let report = Report {
            dpub_version: "test",
            gpu_acceleration: "CPU only",
            tools: vec![tool],
        };
        assert!(plan_install(&report).is_empty());
    }

    #[test]
    fn plan_skips_unknown_tool_keys() {
        let report = Report {
            dpub_version: "test",
            gpu_acceleration: "CPU only",
            tools: vec![missing("nonsense", "Nonsense")],
        };
        assert!(plan_install(&report).is_empty());
    }

    #[test]
    fn plan_skips_whisper_model() {
        // Whisper model is `dpub setup` territory, not the OS package manager.
        let report = Report {
            dpub_version: "test",
            gpu_acceleration: "CPU only",
            tools: vec![missing("whisper-model", "Whisper transcription")],
        };
        assert!(plan_install(&report).is_empty());
    }
}
