use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::ExitCode;

use graphql_schema_check::{compare_schemas, format_report};

const USAGE: &str = "usage: graphql-schema-tools check|update <baseline.graphql> <proposed.graphql> [--summary <file>] [--reason <text>]";

#[derive(Default)]
struct Options {
    summary: Option<String>,
    reason: Option<String>,
    paths: Vec<String>,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--summary" => {
                options.summary = Some(
                    args.get(index + 1)
                        .ok_or("--summary requires a path")?
                        .clone(),
                );
                index += 2;
            }
            "--reason" => {
                options.reason = Some(
                    args.get(index + 1)
                        .ok_or("--reason requires a value")?
                        .clone(),
                );
                index += 2;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option `{flag}`\n{USAGE}"))
            }
            path => {
                options.paths.push(path.to_string());
                index += 1;
            }
        }
    }
    if options.paths.len() != 2 {
        return Err(USAGE.to_string());
    }
    Ok(options)
}

fn read(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))
}

fn append_summary(path: &str, report: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("cannot open summary {path}: {error}"))?;
    writeln!(file, "### GraphQL schema compatibility\n\n{report}")
        .map_err(|error| format!("cannot write summary {path}: {error}"))
}

fn with_approval_header(sdl: &str, reason: &str) -> String {
    let mut body = sdl.trim_start_matches('\u{feff}').trim_start();
    loop {
        let Some(line) = body.lines().next() else {
            break;
        };
        let is_header = line.starts_with("# GraphQL schema baseline approved")
            || line.starts_with("# Regenerate with `graphql-schema-tools update`");
        if !is_header {
            break;
        }
        body = body[line.len()..].trim_start_matches('\n').trim_start();
    }

    format!(
        "# GraphQL schema baseline approved with: {}\n# Regenerate with `graphql-schema-tools update`; breaking changes require review.\n{}",
        reason.trim().replace('\n', " "),
        body
    )
}

fn run(args: &[String]) -> Result<(), String> {
    let command = args.get(1).map(String::as_str).ok_or(USAGE)?;
    if !matches!(command, "check" | "update") {
        return Err(USAGE.to_string());
    }
    let options = parse_args(args)?;
    let baseline_path = &options.paths[0];
    let proposed_path = &options.paths[1];

    let baseline_text = read(baseline_path)?;
    let proposed_text = read(proposed_path)?;
    let changes = compare_schemas(&baseline_text, &proposed_text)?;
    let report = format_report(&changes);
    if let Some(summary) = &options.summary {
        append_summary(summary, &report)?;
    }

    match command {
        "check" => {
            println!("{report}");
            if changes.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "graphql schema compatibility check failed: {} breaking change(s)",
                    changes.len()
                ))
            }
        }
        "update" => {
            if changes.is_empty() {
                fs::write(baseline_path, &proposed_text)
                    .map_err(|error| format!("cannot write {baseline_path}: {error}"))?;
                println!("{report}");
                println!("Baseline synchronized; no breaking changes required approval.");
                return Ok(());
            }

            let reason = options
                .reason
                .as_deref()
                .map(str::trim)
                .filter(|reason| !reason.is_empty())
                .ok_or("--reason is required when approving breaking schema changes")?;
            let approved = with_approval_header(&proposed_text, reason);
            fs::write(baseline_path, approved)
                .map_err(|error| format!("cannot write {baseline_path}: {error}"))?;
            println!("{report}");
            println!("Baseline approved with reason: {reason}");
            println!("Commit both the proposed schema and the updated baseline; the baseline diff must be reviewed.");
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
