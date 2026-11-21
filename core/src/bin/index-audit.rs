//! Turns a statistics snapshot (see `scripts/index_audit/collect.sql`) into the
//! weekly index report.
//!
//! ```text
//! index-audit <snapshot.json> [--date YYYY-MM-DD] [--fail-on-findings]
//! ```
//! The Markdown report goes to stdout. With `--fail-on-findings` the exit code is
//! 1 when anything was found.

use std::process::ExitCode;
use std::{env, fs};

use Perigee_core::index_audit::{audit, render_markdown, Snapshot, Thresholds};

fn run(args: &[String]) -> Result<bool, String> {
    let path = args.get(1).ok_or("usage: index-audit <snapshot.json> [--date YYYY-MM-DD] [--fail-on-findings]")?;
    let date = args
        .iter()
        .position(|a| a == "--date")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "undated".to_string());

    let text = fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let snapshot: Snapshot = serde_json::from_str(&text).map_err(|e| format!("{path} is not a valid snapshot: {e}"))?;

    let report = audit(&snapshot, &Thresholds::default());
    println!("{}", render_markdown(&report, &date));
    Ok(!report.is_clean())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match run(&args) {
        Ok(found) if found && args.iter().any(|a| a == "--fail-on-findings") => ExitCode::FAILURE,
        Ok(_) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
